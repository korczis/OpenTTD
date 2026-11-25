// Re-plan the running bot every game month (RESEARCH-ONLY,
// research/mechanisms/planner-and-algoglyph.md "Replanning while the game runs").
//
// Run it as a service on the Scripts page (/scripts/replan) or:
//   curl -X POST 127.0.0.1:8800/api/services/start -d '{"name":"replan"}'
//
// Once a game month (EVERY_MONTHS), one plan for every running company of AI (step 3), then for
// each of them:
//   1. The company's state, from the engine: the last four quarters of income, expenses, company
//      value, performance rating and cargo delivered (ScriptCompany through MCP), with the trend
//      between the last two, and its rank by value among all companies from the panel's DuckDB
//      store (samples of this run, ADR 0033).
//   2. The competition: every company's last quarter, and per transport type (train, road,
//      aircraft, ship) its vehicles and their profit last year (openttd://vehicles). For each type
//      it names the company that earns most per vehicle and how far this company is from it, so
//      the plan says where the others win and where this company is ahead.
//   3. A fresh plan on today's map and money (api_botPlanned: mcp-plan's portfolio within the loan
//      limit, rail lines priced on the map by AlgoGlyph's track-planning, the knapsack by
//      AlgoGlyph), one call for all the companies. The panel publishes it as plan.portfolio and
//      plan.bot_settings, so it is in the store and on the bus as well. Each company's settings are
//      its own entry of bot_settings.companies: no two companies of the AI are handed one producer,
//      and none a producer where another company already has a rail station
//      (research/incident-2026-11-22-sibling-industries.md). A company without an entry is skipped.
//   4. The switch, modelled before it is proposed. AlgoGlyph's bayes-update turns this company's
//      own record (lines a year old or more that earned, and those that lost) into the chance that
//      a new line pays, with a credible interval. The record is kept per mode (rail, road, air,
//      water, by the vehicles' type), and a plan is judged by the modes of the lines it adds: in
//      game 10487 a pooled record of "3 of 5 lines earned" (two losing bus lines) held a rail plan
//      while the trains earned 35.5k a vehicle. Where it adds lines of two modes, the weaker one
//      decides; a plan that adds no line is judged on the whole record, as before. AlgoGlyph's
//      pert-monte-carlo then draws the months the new portfolio needs to pay back its investment:
//      optimistic at the interval's upper end, likely at its mean, pessimistic at its lower end.
//      The monthly profit it pays back from is the planner's price of each chosen line: its
//      freight price in the company's plan (a freight line without one is another company's and
//      counts 0), else the monthly profit above 0 of the option the planner chose for it (the portfolio's `monthly_profit`, a passenger pair's). A line with
//      neither counts at this company's realised profit per vehicle of its earning lines of that
//      mode a year old (last year's, over 12), times the vehicles the option buys (1 when it does
//      not say): the profit of a line that pays, since the bayes-update above already weighs the
//      chance that it does, and per vehicle, since a mature line's fleet is not a new line's. In
//      game 21647 every round predicted 0 for its passenger plan, so none was ever modelled
//      ("no payback to model") while 11 of 12 lines earned. A mode with no earning line in the
//      record still counts 0.
//      The plan is proposed only when the lower end is at least MIN_PAYS and 80 % of the draws pay
//      back within HORIZON_MONTHS. Otherwise the round says why it holds the current plan.
//   5. A proposed plan goes through build_plan::apply_to that company: only the settings that
//      differ from its bot's live ones, each an openttd.ai.set_setting that waits for the owner's
//      approval, in one call (one plan). This script approves nothing. Before that the difference
//      is guarded by bot_floors.rhai: a line's settings are filed together, and a plan never drops
//      PrismaticAI's autopilot floors. A plan turns the bot "planned", and then main.nut no longer
//      lifts own_lines to 10. In game 10487 the deadline approved such a plan on 1952-04-12, and
//      the bot ran on its live own_lines=4 until the end (2114). So a plan that makes the bot
//      planned also carries each floor its live settings would lose.
//   Why a round holds (another planner, approvals still waiting, or the model) is printed and
//   written as `held` into the record of step 6; null when it proposed or had nothing to change.
//   6. The whole round (state, trend, competition, plan, model, verdict) goes on the shared
//      blackboard as `replan.<company>`; every write is a rhai.shared event, so it lands on the
//      bus and in the store. It also goes to the Majordomus mesh as a `plan` record when the
//      panel has a mesh session ($MAJORDOMUS_URL, $MAJORDOMUS_SESSION); without one it prints why.
//
// Nothing here decides a route or a number by itself. The plan is the planner's and AlgoGlyph's,
// the figures are the engine's, and the company is the running bot's. The setting names are the
// ones the bot declares. Edit the constants while it runs; the service reloads within a second.
import "ttd" as ttd;
import "build_plan" as bp;
import "fr_author" as fa;
import "bot_floors" as bf;

// The AI whose companies are re-planned: every running company of it (/api/bots).
const AI = "PrismaticAI";
// This service's name for the planner key planner.<company> (fr_author.rhai): while the key names
// another writer (Battlebrain's directive during a cutover), a round proposes nothing.
const WRITER = "replan";
// Game months between two plans, and seconds between two looks at the date.
const EVERY_MONTHS = 1;
const ROUND_S = 20;
// Rail lines the planner prices on the map with AlgoGlyph (0 to 16), and how long it may take.
const TRACK = 16;
const PLAN_TIMEOUT_S = 900;
// The switch rule: the credible lower bound of "a new line pays", and the payback horizon.
const MIN_PAYS = 0.5;
const HORIZON_MONTHS = 24;
const CREDIBLE = 0.9;
// A line counts for the record once its vehicles are this many days old.
const RECORD_AGE_DAYS = 365;
// One round and exit, for a check; REPLAN_ONCE=1 in the environment does the same.
const ONCE = false;

const TYPES = ["train", "road", "aircraft", "ship"];
// The mode a vehicle type serves, as bot_floors.rhai names the modes of the lines a plan adds.
const MODE_OF_TYPE = #{ train: "rail", road: "road", aircraft: "air", ship: "water" };
// The record's mode of a portfolio line's mode (mcp-plan's MODE_NAMES and FREIGHT_MODES).
const MODE_OF_PLAN = #{ bus: "road", truck: "road", rail: "rail", freight_rail: "rail", air: "air", ship: "water" };

// The paths a plan record claims on the mesh, which refuses a claim of none: a plan changes the
// AI settings PrismaticAI declares there.
const RECORD_SCOPE = ["tools/research/prismatic-ai/PrismaticAI/info.nut"];

fn bots() {
    let out = [];
    for r in api_bots().running ?? [] {
        if r.ai == global::AI { out.push(r); }
    }
    out
}

// A company id as an integer, whichever number type the JSON gave.
fn id_of(x) { if type_of(x) == "f64" { x.to_int() } else { x } }

// One plan for every company of the AI (step 3). The planner prices up to TRACK rail lines with
// AlgoGlyph: minutes, not seconds, on a busy machine (3:10 at load 50, 2026-11-22), so this one call
// gets PLAN_TIMEOUT_S.
fn plan_all() {
    let before = http_timeout(global::PLAN_TIMEOUT_S);
    let p = ();
    try { p = api_botPlanned(#{ ai: global::AI, track: global::TRACK }); } catch (err) { http_timeout(before); throw err; }
    http_timeout(before);
    if "http_error" in p { throw `planner: ${p.http_error} ${p.body}`; }
    p
}

// The panel's plan for company `id`: its entry of bot_settings.companies, () when it has none.
fn plan_of(p, id) {
    for c in (p.bot_settings ?? #{}).companies ?? [] {
        if id_of(c.company) == id_of(id) { return c; }
    }
    ()
}

// "1951-09-10" -> months since year 0, so two dates subtract.
fn month_of(date) {
    let p = `${date}`.split("-");
    if p.len() < 2 { return 0; }
    parse_int(p[0]) * 12 + parse_int(p[1])
}

// One named query of the store (tools/research/mcp-rhai/sql/queries/<name>.sql) with its
// parameters bound by DuckDB: no SQL is written or joined here. GET /api/db/named lists them.
fn rows_of(name, params) {
    let r = api_dbNamedRun(#{ name: name, params: params });
    if "error" in r { throw `store: ${r.error}`; }
    if "http_error" in r { throw `store: ${r.http_error} ${r.body}`; }
    r.rows ?? []
}

// One ScriptCompany figure of a company for a quarter (1 = the last finished one).
fn quarterly(fname, company, q) {
    let r = Fn(fname).call(#{ args: [company, q] });
    if r.ok ?? false { r.result } else { () }
}

// The last four finished quarters of a company, newest first, and the trend between the last two.
fn finances(company) {
    let qs = [];
    for q in 1..=4 {
        qs.push(#{
            income: quarterly("openttd_api_ScriptCompany_GetQuarterlyIncome", company, q),
            expenses: quarterly("openttd_api_ScriptCompany_GetQuarterlyExpenses", company, q),
            value: quarterly("openttd_api_ScriptCompany_GetQuarterlyCompanyValue", company, q),
            rating: quarterly("openttd_api_ScriptCompany_GetQuarterlyPerformanceRating", company, q),
            cargo: quarterly("openttd_api_ScriptCompany_GetQuarterlyCargoDelivered", company, q) });
    }
    let trend = #{};
    for k in ["income", "expenses", "value", "rating", "cargo"] {
        let a = qs[0][k]; let b = qs[1][k];
        trend[k] = if a == () || b == () { () } else { a - b };
    }
    #{ quarters: qs, trend: trend }
}

// The company's rank by value in this run and its last months' value changes, from the named
// queries company_rank and company_trend over the store's views.
const TREND_MONTHS = 6;

fn rank(company) {
    let me = rows_of("company_rank", #{ company: company });
    let months = [];
    for m in rows_of("company_trend", #{ company: company, months: global::TREND_MONTHS }) {
        months.push(#{ month: m[0], value: m[1], value_change: m[2], money: m[3], money_change: m[4], vehicles: m[5] });
    }
    if me.is_empty() { return #{ rank: 0, of: 0, value: 0, date: "", months: months }; }
    #{ rank: me[0][0], of: me[0][1], value: me[0][2], date: me[0][3], months: months }
}

fn vehicles() { read("openttd://vehicles").data.vehicles ?? [] }

// Per company and transport type: vehicles, those old enough to count, and their profit last year.
fn fleet_stats(vs) {
    let out = #{};
    for v in vs {
        let c = `${v.owner}`;
        if !(c in out) { out[c] = #{}; }
        let t = v.type;
        if !(t in out[c]) { out[c][t] = #{ vehicles: 0, counted: 0, profit: 0 }; }
        let s = out[c][t];
        s.vehicles += 1;
        if v.age >= global::RECORD_AGE_DAYS { s.counted += 1; s.profit += v.profit_last_year; }
        out[c][t] = s;
    }
    out
}

// Where the competition wins: per type, the company earning most per counted vehicle against us.
fn competition(company, stats, names) {
    let me = `${company}`;
    let rows = [];
    for t in global::TYPES {
        let best = (); let best_avg = 0.0;
        for c in stats.keys() {
            let s = stats[c][t] ?? ();
            if s == () || s.counted == 0 { continue; }
            let avg = s.profit.to_float() / s.counted.to_float();
            if best == () || avg > best_avg { best = c; best_avg = avg; }
        }
        if best == () { continue; }
        let mine = stats[me][t] ?? ();
        let my_avg = if mine == () || mine.counted == 0 { () } else { mine.profit.to_float() / mine.counted.to_float() };
        rows.push(#{ type: t, leader: names[best] ?? best, leader_per_vehicle: best_avg.round(),
                     ours_per_vehicle: if my_avg == () { () } else { my_avg.round() },
                     ours_vehicles: if mine == () { 0 } else { mine.vehicles },
                     we_lead: best == me });
    }
    rows
}

// This company's record: each line (vehicles sharing their orders) old enough, earned or lost,
// in all and per mode (`by_mode`, MODE_OF_TYPE).
fn record_of(company, vs) {
    let lines = #{};
    let vehicles = #{};
    let mode_of = #{};
    for v in vs {
        if v.owner != company || v.age < global::RECORD_AGE_DAYS || v.orders.is_empty() { continue; }
        let stops = [];
        for o in v.orders { stops.push(`${o.destination}`); }
        stops.sort();
        let k = `${v.type}:${stops}`;
        lines[k] = (lines[k] ?? 0) + v.profit_last_year;
        vehicles[k] = (vehicles[k] ?? 0) + 1;
        mode_of[k] = global::MODE_OF_TYPE[v.type] ?? v.type;
    }
    let won = 0; let lost = 0;
    let by_mode = #{};
    for k in lines.keys() {
        let m = mode_of[k];
        if !(m in by_mode) { by_mode[m] = #{ lines: 0, earned: 0, lost: 0, profit: 0, earned_per_vehicle: 0.0 }; }
        let r = by_mode[m];
        r.lines += 1;
        r.profit += lines[k];
        // An earning line's profit per vehicle: a mature line of four trains says what one train earns.
        let p = if type_of(lines[k]) == "f64" { lines[k] } else { lines[k].to_float() };
        if p > 0.0 { won += 1; r.earned += 1; r.earned_per_vehicle += p / vehicles[k].to_float(); } else { lost += 1; r.lost += 1; }
        by_mode[m] = r;
    }
    #{ lines: lines.len(), earned: won, lost: lost, by_mode: by_mode }
}

// A company's realised monthly profit per vehicle of an earning line of a portfolio line's mode, from
// its record: the mean over that mode's earning lines of last year's profit per vehicle, over 12, a
// whole number; () when the record has no earning line of that mode. Conditional on earning, because
// model()'s bayes-update already weighs the chance that a new line earns; per vehicle, because a
// mature line's fleet is not what a new line buys.
fn realised_monthly(rec, plan_mode) {
    let r = rec.by_mode[global::MODE_OF_PLAN[plan_mode] ?? plan_mode];
    if type_of(r) == "()" || (r.earned ?? 0) == 0 { return (); }
    (r.earned_per_vehicle / r.earned.to_float() / 12.0).floor().to_int()
}

// The predicted monthly profit of exactly the lines the investment buys. Each chosen line is matched
// to its priced entry in the company's plan (`priced`, producer>acceptor:cargo -> monthly profit);
// a freight line without it is another company's and adds nothing; a passenger line counts at the
// planner's own monthly profit of its chosen option when that is above 0, else at the record's realised profit per vehicle (realised_monthly) times the vehicles its
// option buys (1 when it does not say), and one without any adds nothing. `earned` names, per mode,
// how many earning lines a record price came from.
fn predicted_monthly(chosen, priced, rec) {
    let out = #{ monthly: 0, planner: 0, record: 0, unpriced: 0, earned: #{} };
    for c in chosen {
        let id = |x| { let s = `${x.id ?? x}`; let p = s.split(":"); p[p.len() - 1] };
        let k = `${id.call(c.from)}>${id.call(c.to)}:${c.cargo_id ?? ""}`;
        if k in priced { out.monthly += priced[k].to_int(); out.planner += 1; continue; }
        // A freight line (the planner's kind) is this company's only when its own plan prices it
        // (another company's producer otherwise), so it is never priced from the option or the record.
        if c.kind == "freight" { out.unpriced += 1; continue; }
        let own = c.monthly_profit;
        if (type_of(own) == "f64" || type_of(own) == "i64") && own > 0 { out.monthly += own.to_int(); out.planner += 1; continue; }
        let r = realised_monthly(rec, c.mode);
        if type_of(r) == "()" { out.unpriced += 1; continue; }
        let n = c.vehicles;
        let n = if type_of(n) == "i64" && n > 0 { n } else if type_of(n) == "f64" && n >= 1.0 { n.to_int() } else { 1 };
        out.monthly += r * n;
        out.record += 1;
        let m = global::MODE_OF_PLAN[c.mode] ?? c.mode;
        out.earned[m] = rec.by_mode[m].earned;
    }
    out
}

// bayes-update on one record (#{earned, lost}).
fn pays(r) {
    kernel("bayes-update", #{ successes: r.earned, failures: r.lost, prior: "jeffreys", credible: global::CREDIBLE })
}

// The switch model: bayes-update on the record of each mode the plan adds lines of (`modes`, from
// bot_floors::guard), the weakest deciding, or on the whole record when it adds none; then
// pert-monte-carlo on the payback.
fn model(rec, portfolio, monthly, modes) {
    let judged = #{};
    let by = "all";
    let b = ();
    for m in modes {
        let r = rec.by_mode[m] ?? #{ lines: 0, earned: 0, lost: 0 };
        let bm = pays(r);
        judged[m] = #{ lines: r.lines, earned: r.earned, lost: r.lost, lower: bm.lower, mean: bm.mean, upper: bm.upper };
        if type_of(b) == "()" || bm.lower < b.lower { b = bm; by = m; }
    }
    if type_of(b) == "()" { b = pays(rec); }
    let investment = (portfolio.investment_k ?? 0) * 1000;
    let out = #{ pays_mean: b.mean, pays_lower: b.lower, pays_upper: b.upper, judged_by: by, by_mode: judged,
                 investment: investment, monthly_profit: monthly, payback: () };
    if monthly > 0 && investment > 0 && b.lower > 0 {
        let m = investment.to_float() / monthly.to_float();
        let p = kernel("pert-monte-carlo", #{ tasks: [#{ name: "payback", a: m / b.upper, m: m / b.mean,
                                                         b: m / b.lower, after: [] }], runs: 5000 });
        out.payback = #{ p50: p.p50.round(), p80: p.p80.round(), p90: p.p90.round() };
    }
    out.adopt = out.payback != () && b.lower >= global::MIN_PAYS && out.payback.p80 <= global::HORIZON_MONTHS;
    out.why = if out.payback == () { "no payback to model (no investment, no profit, or no line in the record pays)" }
              else if b.lower < global::MIN_PAYS { `a new ${if by == "all" { "" } else { by + " " }}line pays with ${b.lower} at the ${global::CREDIBLE} lower bound, below ${global::MIN_PAYS}` }
              else if out.payback.p80 > global::HORIZON_MONTHS { `80 % of draws pay back in ${out.payback.p80} months, beyond ${global::HORIZON_MONTHS}` }
              else { `pays with at least ${b.lower}; 80 % pay back within ${out.payback.p80} months` };
    out
}

// How many setting changes for this company wait for the owner's approval now (the game server's
// pending approvals, status().pending; one setting, or a plan of settings). While any wait, a
// round proposes nothing new.
fn waiting_for_owner(company) {
    let n = 0;
    for d in status().pending ?? [] {
        let c = (d.args ?? #{}).company;
        if type_of(c) == "f64" { c = c.to_int(); }
        if (d.tool == "openttd.ai.set_setting" || d.tool == "openttd.ai.set_settings") && c == company { n += 1; }
    }
    n
}

// "a=1,b=2" -> #{a: 1, b: 2}
fn settings_of(s) {
    let m = #{};
    for kv in `${s}`.split(",") {
        let p = kv.split("=");
        if p.len() == 2 { let k = p[0]; let v = p[1]; k.trim(); v.trim(); m[k] = parse_int(v); }
    }
    m
}

// The planned settings whose value differs from the bot's live one.
fn changes(planned, live) {
    let now = #{};
    for s in live { now[s.name] = s.value; }
    let out = #{};
    for k in planned.keys() {
        if !(k in now) || now[k] != planned[k] { out[k] = planned[k]; }
    }
    out
}

fn say(text) {
    text.replace("\"", "'");
    con_say(`"[replan] ${text}"`);
}

fn record(company, summary) {
    shared_set(`replan.${company}`, summary);
    try {
        mesh_record_write(#{ kind: "plan", task: `openttd:replan:${company}`, scope: global::RECORD_SCOPE,
            title: `replan of company ${company + 1} on ${summary.date}: ${if summary.model.adopt { "switch" } else { "hold" }}`,
            body: to_json(summary) });
    } catch (err) {
        print(`mesh: ${err}`);
    }
}

fn plan_round(b, p) {
    let mine = plan_of(p, b.id);
    if type_of(mine) == "()" {
        print(`== company ${b.id + 1} (${b.name}): the plan has no entry for it; nothing proposed`);
        return ();
    }
    let r = rank(b.id);
    let f = finances(b.id);
    let q = f.quarters[0];
    print(`== ${r.date} company ${b.id + 1} (${b.name}): rank ${r.rank}/${r.of}, value ${r.value}; last quarter income ${q.income}, expenses ${q.expenses}, rating ${q.rating}, cargo ${q.cargo}; trend ${f.trend}`);

    let names = #{};
    let others = [];
    for c in read("openttd://companies").data.companies ?? [] {
        names[`${c.id}`] = c.name;
        if c.id != b.id {
            let fq = finances(c.id).quarters[0];
            others.push(#{ company: c.id, name: c.name, value: c.company_value, income: fq.income,
                           expenses: fq.expenses, rating: fq.rating, cargo: fq.cargo });
        }
    }
    let vs = vehicles();
    let comp = competition(b.id, fleet_stats(vs), names);
    for row in comp {
        print(`   ${row.type}: ` + if row.we_lead { `we lead with ${row.ours_per_vehicle} a vehicle` }
              else { `${row.leader} earns ${row.leader_per_vehicle} a vehicle, we ${row.ours_per_vehicle ?? "none"} (${row.ours_vehicles} vehicle(s))` });
    }

    let portfolio = p.portfolio ?? #{};
    // The predicted monthly profit of exactly the lines the investment buys (predicted_monthly):
    // the plan's price of a chosen line, else the record's realised profit per earning line of its
    // mode, else nothing.
    let priced = #{};
    for l in mine.freight ?? [] { priced[`${l.from}>${l.to}:${l.cargo_id}`] = l.monthly_profit ?? 0.0; }
    let chosen = [];
    for c in portfolio.chosen ?? [] {
        chosen.push(`${c.mode} ${c.from.name ?? c.from} -> ${c.to.name ?? c.to}` + if "cargo" in c { ` (${c.cargo})` } else { "" });
    }
    let rec = record_of(b.id, vs);
    let pm = predicted_monthly(portfolio.chosen ?? [], priced, rec);
    let monthly = pm.monthly;
    print(`   plan: ${chosen.len()} line(s), investment ${portfolio.investment_k ?? 0}k, predicted ${monthly} a month` +
          if pm.record > 0 { ` (${pm.record} line(s) without the planner's price at the record's realised profit per vehicle of their mode's earning lines)` } else { "" } +
          if pm.unpriced > 0 { ` (${pm.unpriced} line(s) without a price count as 0)` } else { "" } +
          ` (${portfolio.solver ?? "?"}; rail lines with a route on the map: ${(portfolio.priced ?? #{}).with_route ?? "?"} of ${(portfolio.priced ?? #{}).lines ?? "?"}): ${chosen}`);

    let g = bf::guard(b.settings ?? [], changes(settings_of(mine.start_ai), b.settings ?? []));
    let diff = g.settings;
    for x in g.raised { print(`   floor: ${x.name} ${x.from} -> ${x.to} (PrismaticAI's autopilot floor, which a planned bot no longer applies)`); }
    if !g.grouped.is_empty() { print(`   filed with their line: ${g.grouped}`); }

    let m = model(rec, portfolio, monthly, g.added_modes);
    print(`   record: ${rec.earned} of ${rec.lines} line(s) a year old earned` +
          if m.judged_by == "all" { "" } else { `; judged by ${m.judged_by}: ${m.by_mode[m.judged_by].earned} of ${m.by_mode[m.judged_by].lines}` } +
          `; ${m.why}`);

    // The floors a planned bot has already lost (game 10487 from 1952-04-15) are no bet on a new
    // line: while the model holds the plan, they are proposed alone (the owner's approval still
    // decides them).
    let restore = #{};
    for x in g.lost { restore[x.name] = x.to; }
    let applied = #{ pending: [], refused: [], lines: [] };
    let waiting = waiting_for_owner(b.id);
    let by = fa::refusal(b.id, global::WRITER);
    let held = ();
    if type_of(by) != "()" {
        // Another planner holds the company (e.g. Battlebrain's directive): one writer per company.
        held = by.why;
    } else if waiting > 0 {
        // The owner has not answered the last proposal yet; a new one would only pile up beside it.
        held = `${waiting} earlier change(s) for company ${b.id + 1} still wait for the owner's approval`;
    } else if !m.adopt && !diff.is_empty() {
        held = `the model holds the current plan (${diff.len()} setting(s) would change): ${m.why}`;
    }
    let send = if type_of(held) == "()" { diff } else if type_of(by) == "()" && waiting == 0 { restore } else { #{} };
    if type_of(held) != "()" { print(`   holding: ${held}`); }
    if !send.is_empty() {
        applied = bp::apply_to(#{ build: #{ settings: send } }, global::WRITER, b);
        for l in applied.lines { print(`   ${l}`); }
        let why = if type_of(held) == "()" { m.why } else { "they restore PrismaticAI's autopilot floors, which the bot lost when it became planned" };
        say(`new plan for ${b.name}: ${send.len()} setting(s) wait for your approval (${applied.pending}); ${why}`);
    } else if type_of(held) == "()" {
        print("   the plan equals the bot's live settings");
    }

    let summary = #{ date: r.date, company: b.id, bot: b.name, rank: r.rank, of: r.of, value: r.value, value_by_month: r.months,
                     finances: f, competitors: others, competition: comp, portfolio: chosen,
                     investment_k: portfolio.investment_k, predicted_monthly: monthly,
                     monthly_from: #{ planner: pm.planner, record: pm.record, unpriced: pm.unpriced, earned: pm.earned }, record: rec,
                     model: m, floors: #{ raised: g.raised, lost: g.lost, grouped: g.grouped, planned_before: g.planned_before,
                                                  planned_after: g.planned_after, modes: g.added_modes },
                     proposed: if type_of(by) == "()" { diff.keys() } else { [] }, sent: send.keys(), held: held, held_by: by,
                     pending: applied.pending, refused: applied.refused };
    record(b.id, summary);
    summary
}

// One round: one plan for every running company of the AI, then each company's steps 1, 2 and 4
// to 6; a company whose step fails does not stop the others. False while none runs. Not named
// `round`: a script fn of that name would also answer `x.round()` on a number, and recurse.
fn one_round() {
    let bs = bots();
    if bs.is_empty() {
        print(`no running ${global::AI} yet`);
        return false;
    }
    let p = plan_all();
    for b in bs {
        try { plan_round(b, p); } catch (err) { print(`replan: company ${b.id + 1}: ${err}`); }
    }
    true
}

let last = ();
let once = ONCE || env("REPLAN_ONCE") == "1";
loop {
    try {
        let s = ttd::state();
        if (type_of(last) == "()" || month_of(s.date) - month_of(last) >= EVERY_MONTHS) && one_round() {
            last = s.date;
        }
    } catch (err) {
        print(`replan: ${err}`);
    }
    if once { break; }
    sleep(ROUND_S * 1000);
}
