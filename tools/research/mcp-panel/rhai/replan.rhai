// Re-plan the running bot every game month (RESEARCH-ONLY,
// research/mechanisms/planner-and-algoglyph.md "Replanning while the game runs").
//
// Run it as a service on the Scripts page (/scripts/replan) or:
//   curl -X POST 127.0.0.1:8800/api/services/start -d '{"name":"replan"}'
//
// Once a game month (EVERY_MONTHS), for the first running company of AI:
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
//      AlgoGlyph). The panel publishes it as plan.portfolio and plan.bot_settings, so it is in the
//      store and on the bus as well.
//   4. The switch, modelled before it is proposed. AlgoGlyph's bayes-update turns this company's
//      own record (lines a year old or more that earned, and those that lost) into the chance that
//      a new line pays, with a credible interval. AlgoGlyph's pert-monte-carlo then draws the
//      months the new portfolio needs to pay back its investment: optimistic at the interval's
//      upper end, likely at its mean, pessimistic at its lower end. The plan is proposed only when
//      the lower end is at least MIN_PAYS and 80 % of the draws pay back within HORIZON_MONTHS.
//      Otherwise the round says why it holds the current plan.
//   5. A proposed plan goes through build_plan::apply: only the settings that differ from the
//      bot's live ones, each an openttd.ai.set_setting that waits for the owner's approval. This
//      script approves nothing.
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

// The AI whose company is re-planned: the first running company of it (/api/bots).
const AI = "PrismaticAI";
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

fn bot() {
    for r in api_bots().running ?? [] {
        if r.ai == global::AI { return r; }
    }
    ()
}

// "1951-09-10" -> months since year 0, so two dates subtract.
fn month_of(date) {
    let p = `${date}`.split("-");
    if p.len() < 2 { return 0; }
    parse_int(p[0]) * 12 + parse_int(p[1])
}

fn rows_of(sql) {
    let r = api_dbQuery(#{ sql: sql });
    if "error" in r { throw `store: ${r.error}`; }
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

// The company's rank by value among all companies, from this run's latest samples in the store.
fn rank(company) {
    let run = "(SELECT max(started_ms) FROM runs)";
    let latest = rows_of(`SELECT company, value, game_date FROM samples s WHERE ts_ms >= ${run}
        AND ts_ms = (SELECT max(ts_ms) FROM samples t WHERE t.company = s.company AND t.ts_ms >= ${run})
        ORDER BY value DESC`);
    let i = 0;
    for r in latest { i += 1; if r[0] == company { return #{ rank: i, of: latest.len(), value: r[1], date: r[2] }; } }
    #{ rank: 0, of: latest.len(), value: 0, date: "" }
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

// This company's record: each line (vehicles sharing their orders) old enough, earned or lost.
fn record_of(company, vs) {
    let lines = #{};
    for v in vs {
        if v.owner != company || v.age < global::RECORD_AGE_DAYS || v.orders.is_empty() { continue; }
        let stops = [];
        for o in v.orders { stops.push(`${o.destination}`); }
        stops.sort();
        let k = `${v.type}:${stops}`;
        lines[k] = (lines[k] ?? 0) + v.profit_last_year;
    }
    let won = 0; let lost = 0;
    for k in lines.keys() { if lines[k] > 0 { won += 1; } else { lost += 1; } }
    #{ lines: lines.len(), earned: won, lost: lost }
}

// The switch model: bayes-update on the record, then pert-monte-carlo on the payback.
fn model(rec, portfolio, monthly) {
    let b = kernel("bayes-update", #{ successes: rec.earned, failures: rec.lost, prior: "jeffreys",
                                      credible: global::CREDIBLE });
    let investment = (portfolio.investment_k ?? 0) * 1000;
    let out = #{ pays_mean: b.mean, pays_lower: b.lower, pays_upper: b.upper,
                 investment: investment, monthly_profit: monthly, payback: () };
    if monthly > 0 && investment > 0 && b.lower > 0 {
        let m = investment.to_float() / monthly.to_float();
        let p = kernel("pert-monte-carlo", #{ tasks: [#{ name: "payback", a: m / b.upper, m: m / b.mean,
                                                         b: m / b.lower, after: [] }], runs: 5000 });
        out.payback = #{ p50: p.p50.round(), p80: p.p80.round(), p90: p.p90.round() };
    }
    out.adopt = out.payback != () && b.lower >= global::MIN_PAYS && out.payback.p80 <= global::HORIZON_MONTHS;
    out.why = if out.payback == () { "no payback to model (no investment, no profit, or no line in the record pays)" }
              else if b.lower < global::MIN_PAYS { `a new line pays with ${b.lower} at the ${global::CREDIBLE} lower bound, below ${global::MIN_PAYS}` }
              else if out.payback.p80 > global::HORIZON_MONTHS { `80 % of draws pay back in ${out.payback.p80} months, beyond ${global::HORIZON_MONTHS}` }
              else { `pays with at least ${b.lower}; 80 % pay back within ${out.payback.p80} months` };
    out
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
        mesh_record_write(#{ kind: "plan", task: `openttd:replan:${company}`, scope: [],
            title: `replan of company ${company + 1} on ${summary.date}: ${if summary.model.adopt { "switch" } else { "hold" }}`,
            body: to_json(summary) });
    } catch (err) {
        print(`mesh: ${err}`);
    }
}

fn plan_round(b) {
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

    // The planner prices up to TRACK rail lines with AlgoGlyph: minutes, not seconds, on a busy
    // machine (3:10 at load 50, 2026-11-22), so this one call gets PLAN_TIMEOUT_S.
    let before = http_timeout(global::PLAN_TIMEOUT_S);
    let p = ();
    try { p = api_botPlanned(#{ ai: global::AI, track: global::TRACK }); } catch (err) { http_timeout(before); throw err; }
    http_timeout(before);
    if "http_error" in p { throw `planner: ${p.http_error} ${p.body}`; }
    let portfolio = p.portfolio ?? #{};
    // The predicted monthly profit of exactly the lines the investment buys: each chosen freight
    // line matched to its priced entry in bot_settings by producer, acceptor and cargo. A chosen
    // line without a priced entry adds nothing, so the payback errs on the long side.
    let priced = #{};
    for l in (p.bot_settings ?? #{}).freight ?? [] { priced[`${l.from}>${l.to}:${l.cargo_id}`] = l.monthly_profit ?? 0.0; }
    let chosen = [];
    let monthly = 0;
    let unpriced = 0;
    for c in portfolio.chosen ?? [] {
        chosen.push(`${c.mode} ${c.from.name ?? c.from} -> ${c.to.name ?? c.to}` + if "cargo" in c { ` (${c.cargo})` } else { "" });
        let id = |x| { let s = `${x.id ?? x}`; let p = s.split(":"); p[p.len() - 1] };
        let k = `${id.call(c.from)}>${id.call(c.to)}:${c.cargo_id ?? ""}`;
        if k in priced { monthly += priced[k].to_int(); } else { unpriced += 1; }
    }
    print(`   plan: ${chosen.len()} line(s), investment ${portfolio.investment_k ?? 0}k, predicted ${monthly} a month` +
          if unpriced > 0 { ` (${unpriced} line(s) without a price count as 0)` } else { "" } +
          ` (${portfolio.solver ?? "?"}; rail lines with a route on the map: ${(portfolio.priced ?? #{}).with_route ?? "?"} of ${(portfolio.priced ?? #{}).lines ?? "?"}): ${chosen}`);

    let rec = record_of(b.id, vs);
    let m = model(rec, portfolio, monthly);
    print(`   record: ${rec.earned} of ${rec.lines} line(s) a year old earned; ${m.why}`);

    let diff = changes(settings_of(p.settings), b.settings ?? []);
    let applied = #{ pending: [], refused: [], lines: [] };
    if m.adopt && !diff.is_empty() {
        applied = bp::apply(#{ build: #{ settings: diff } });
        for l in applied.lines { print(`   ${l}`); }
        say(`new plan for ${b.name}: ${diff.len()} setting(s) wait for your approval (${applied.pending}); ${m.why}`);
    } else if diff.is_empty() {
        print("   the plan equals the bot's live settings");
    } else {
        print(`   holding the current plan: ${diff.len()} setting(s) would change`);
    }

    let summary = #{ date: r.date, company: b.id, bot: b.name, rank: r.rank, of: r.of, value: r.value,
                     finances: f, competitors: others, competition: comp, portfolio: chosen,
                     investment_k: portfolio.investment_k, predicted_monthly: monthly, record: rec,
                     model: m, proposed: diff.keys(), pending: applied.pending, refused: applied.refused };
    record(b.id, summary);
    summary
}

let last = ();
let once = ONCE || env("REPLAN_ONCE") == "1";
loop {
    try {
        let b = bot();
        let s = ttd::state();
        if type_of(b) == "()" {
            print(`no running ${AI} yet`);
        } else if type_of(last) == "()" || month_of(s.date) - month_of(last) >= EVERY_MONTHS {
            plan_round(b);
            last = s.date;
        }
    } catch (err) {
        print(`replan: ${err}`);
    }
    if once { break; }
    sleep(ROUND_S * 1000);
}
