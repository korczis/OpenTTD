// stall: 300
// Capital watch (RESEARCH-ONLY; research/mechanisms/rhai-scripting.md "The capital watch"): does a
// company put the money it earns back to work? The post-mortem of research/analysis-2026-11-23-
// why-prismatic-loses.md found that the decisive live signal was capital NOT put to work: in game
// 12609 PrismaticAI reinvested 62 %, 44 %, then 12 % of what it earned while AAAHogEx reinvested
// 98-100 %; in game 10487 company 1 sat on 165M+ with 42 vehicles for 160 game years. The watchdog
// detects inactivity only and gap.<company> judges value share; nothing measured reinvestment.
//
// Run it as a service on the Scripts page (/scripts/capital_watch) or:
//   curl -X POST 127.0.0.1:<panel>/api/services/start -d '{"name":"capital_watch"}'
//
// Once a game month (the engine's game date, never the wall clock), for EVERY company in the game,
// so that each company is compared with its leader on the same terms:
//   1. The engine's quarterly economy of the last QUARTERS finished quarters (ScriptCompany
//      GetQuarterlyIncome and GetQuarterlyExpenses through MCP, as replan.rhai reads them). In the
//      engine (src/company_cmd.cpp SubtractMoneyFromCompany) income is what vehicles earn, and
//      expenses (negative) are running costs, property maintenance and loan interest only:
//      construction and new vehicles are in neither. Operating cash flow = income + expenses.
//   2. The money each quarter kept: net (money minus loan) at the end of the quarter's last month
//      minus net at the end of the quarter before, from the store's company_month view (named
//      query capital_months: no new ingestion). A loan taken or repaid moves money and loan alike,
//      so it is no spending.
//   3. Implied capex = operating cash flow minus the net kept: the money that left for something
//      outside running costs (vehicles, track, stations, terraforming), less what sales brought
//      in. It is a residual, not a booked figure: a quarter of vehicle sales makes it negative,
//      and that is kept and shown, never clipped. Reinvestment = implied capex / operating cash
//      flow, for a quarter whose cash flow is at least MIN_OCF (a smaller one makes the ratio
//      noise: unknown). The trailing year is the sums over the QUARTERS quarters.
//   4. Idle-capital ratio = (money - loan) / trailing-year income: how many years of income sit in
//      the bank (unknown while that income is under MIN_INCOME).
//   5. Fleet growth: vehicles now minus GROWTH_MONTHS months ago (company_month), against the
//      company's leader: the most valuable OTHER company now (openttd://companies).
// Flags, diagnostic only (they say what the numbers are, not why the company chose them):
//   under_reinvesting  the newest PERSIST finished quarters each reinvested less than the floor;
//                      an unknown quarter ends the streak, it never counts toward it.
//   hoarding           the idle-capital ratio is at least the idle threshold and the fleet grew less
//                      than the leader's (or not at all) over GROWTH_MONTHS months.
// Thresholds are policy (ADR 0042 section 3): the shared key derived.capital_watch.<param> when it
// holds a value in range (published by the self-evaluation service, research/mechanisms/
// service-self-evaluation.md), else the declared cold-start constant; capital.<company>.thresholds
// names which one was used (threshold_source) and why.
//
// Where it goes: one blackboard key per company, capital.<company> (writer capital_watch; a
// rhai.shared event, so on the bus and in the DuckDB store), with ADR 0042's envelope (company,
// period, game_date, writer, version, reads with row counts, unknown), and one printed line in the
// service's log when a flag holds, once per newly finished quarter. It never acts on the game: it
// reads resources and getters, writes the blackboard and prints; it calls no tool, changes no
// setting, files no proposal, chats nothing and approves nothing (boundary B3 of rule
// project.decision-authority-boundaries). It prints one line a look, so the supervisor's stall limit
// on the first line is not reached while the game is paused. POST /api/services/stop {"name": "capital_watch"} stops
// it; CAPITAL_WATCH_ROUNDS ends it after that many looks (0 or unset: never). It adds no compute
// work to a host beyond one store query and 8 getter calls per company a game month.

// This service's name as the writer of capital.<company>, and the envelope's version.
const WRITER = "capital_watch";
const VERSION = 1;
const ROUND_S = 20;
// Finished quarters read from the engine (it keeps MAX_HISTORY_QUARTERS = 24; four is a year), and
// the months of company_month the query returns (the quarters, one more quarter's end, a margin).
const QUARTERS = 4;
const MONTHS = 18;
const GROWTH_MONTHS = 12;
// Newest finished quarters in a row below the floor before under_reinvesting holds.
const PERSIST = 2;
// Cold-start thresholds (declared bootstraps, ADR 0042): a company that keeps more than half of
// what it earns for two quarters, and one with a year of income in the bank that does not grow its
// fleet as its leader does. The derived keys replace them; their bounds keep a broken value out.
const REINVEST_FLOOR = 0.5;
const DERIVED_FLOOR = "derived.capital_watch.reinvestment_floor";
const FLOOR_RANGE = [0.0, 1.5];
const IDLE_YEARS = 1.0;
const DERIVED_IDLE = "derived.capital_watch.idle_years";
const IDLE_RANGE = [0.25, 50.0];
// Materiality: a quarter's operating cash flow, and a year's income, below which a ratio is noise.
const MIN_OCF = 10000;
const MIN_INCOME = 40000;

fn rounds() { let v = env("CAPITAL_WATCH_ROUNDS"); if v == "" { 0 } else { parse_int(v) } }

// A company id as an integer, whichever number type the JSON gave.
fn id_of(x) { if type_of(x) == "f64" { x.to_int() } else { x } }

// A number as a float; () stays ().
fn num(x) { if type_of(x) == "i64" { x.to_float() } else if type_of(x) == "f64" { x } else { () } }

// "1954-10-03" or "1954-10" -> months since year 0 (January = 0), () when it is no date.
fn mi_of(date) {
    let d = `${date ?? ""}`;
    if d.len() < 7 || d.sub_string(4, 1) != "-" { return (); }
    let y = parse_int(d.sub_string(0, 4));
    let m = parse_int(d.sub_string(5, 2));
    if m < 1 || m > 12 { return (); }
    y * 12 + m - 1
}

// Months since year 0 -> "YYYY-MM".
fn month_text(mi) {
    let m = mi % 12 + 1;
    `${mi / 12}-${if m < 10 { "0" } else { "" }}${m}`
}

// The quarter a month lies in, as "1954Q3".
fn quarter_text(mi) { `${mi / 12}Q${(mi % 12) / 3 + 1}` }

// The items as one text, separated by "; ".
fn joined(items) {
    let out = "";
    for t in items { out += if out == "" { `${t}` } else { `; ${t}` }; }
    out
}

// A ratio as a whole percentage, () stays ().
fn pct(x) { if type_of(x) == "f64" { (x * 100.0).round().to_int() } else { () } }

// The getter's answer as a float, () when it failed or gave the engine's -1 (invalid company or
// quarter, src/script/api/script_company.cpp).
fn econ_value(r) {
    if type_of(r) != "map" || !(r.ok ?? false) { return (); }
    let v = num(r.result);
    if v == () || v == -1.0 { () } else { v }
}

// The engine's last QUARTERS finished quarters of a company, newest first: [{income, expenses}].
fn economy(company) {
    let out = [];
    for q in 1..=global::QUARTERS {
        out.push(#{
            income: econ_value(Fn("openttd_api_ScriptCompany_GetQuarterlyIncome").call(#{ args: [company, q] })),
            expenses: econ_value(Fn("openttd_api_ScriptCompany_GetQuarterlyExpenses").call(#{ args: [company, q] })) });
    }
    out
}

// The store's rows of capital_months as maps, grouped by company: "<id>" -> "YYYY-MM" -> row.
// () when the store is off or cannot answer.
fn by_company(answer) {
    if type_of(answer) != "map" || "error" in answer || "http_error" in answer { return (); }
    let cols = answer.columns ?? [];
    let out = #{};
    for r in answer.rows ?? [] {
        let m = #{};
        for i in 0..cols.len() { m[cols[i]] = r[i]; }
        let key = `${id_of(m.company)}`;
        if !(key in out) { out[key] = #{}; }
        let c = out[key];
        c[`${m.month}`] = m;
        out[key] = c;
    }
    out
}

// One threshold, pure: the derived key's value (a number, or a map with value) when it lies in
// range, else the cold-start constant; with its source and why.
fn threshold_of(derived, key, fallback, name, range) {
    let v = derived;
    if type_of(v) == "map" { v = v.value; }
    v = num(v);
    if v != () && v >= range[0] && v <= range[1] {
        return #{ value: v, source: key, bootstrap: false, why: `${key} = ${v}` };
    }
    let why = if type_of(derived) == "()" { `${key} is not published` } else { `${key} = ${derived} is outside ${range[0]}..${range[1]}` };
    #{ value: fallback, source: `${name} (cold start)`, bootstrap: true, why: `${why}: the declared cold start ${fallback}` }
}

// The finished quarters of one company, newest first, pure: the engine's economy (newest first),
// the company's months of company_month ("YYYY-MM" -> row), the game date now.
fn quarters_of(econ, months, date) {
    let now = mi_of(date);
    let out = [];
    if now == () { return out; }
    // The first month of the quarter running now; quarter k ended the month before k-1 quarters back.
    let start = now - now % 3;
    for k in 0..econ.len() {
        let end_mi = start - 3 * k - 1;
        let e = econ[k];
        let row = #{ quarter: quarter_text(end_mi), income: e.income, expenses: e.expenses, ocf: (),
                     net_start: (), net_end: (), net_kept: (), implied_capex: (), reinvestment: (), unknown: () };
        let a = months[month_text(end_mi - 3)];
        let b = months[month_text(end_mi)];
        if type_of(a) == "map" { row.net_start = num(a.net); }
        if type_of(b) == "map" { row.net_end = num(b.net); }
        if e.income == () || e.expenses == () {
            row.unknown = "the engine gave no economy for the quarter";
        } else {
            row.ocf = e.income + e.expenses;
            if row.net_start == () || row.net_end == () {
                row.unknown = `the store holds no month-end net for ${if row.net_start == () { month_text(end_mi - 3) } else { month_text(end_mi) }}`;
            } else {
                row.net_kept = row.net_end - row.net_start;
                row.implied_capex = row.ocf - row.net_kept;
                if row.ocf < global::MIN_OCF.to_float() {
                    row.unknown = `operating cash flow ${row.ocf.to_int()} is under ${global::MIN_OCF}`;
                } else {
                    row.reinvestment = row.implied_capex / row.ocf;
                }
            }
        }
        out.push(row);
    }
    out
}

// The trailing year over the quarters, pure: sums and the ratio when every quarter is known.
fn trailing_of(quarters) {
    let out = #{ quarters: quarters.len(), income: 0.0, ocf: 0.0, implied_capex: 0.0, reinvestment: (), unknown: () };
    let income_known = true;
    for q in quarters {
        if q.income == () { income_known = false; } else { out.income += q.income; }
        if q.implied_capex == () { if out.unknown == () { out.unknown = `${q.quarter}: ${q.unknown}`; } } else { out.ocf += q.ocf; out.implied_capex += q.implied_capex; }
    }
    if !income_known || quarters.is_empty() { out.income = (); }
    if out.unknown == () && quarters.len() > 0 {
        if out.ocf >= global::MIN_OCF.to_float() * quarters.len().to_float() {
            out.reinvestment = out.implied_capex / out.ocf;
        } else {
            out.unknown = `operating cash flow ${out.ocf.to_int()} over the year is under ${global::MIN_OCF} a quarter`;
        }
    }
    if out.unknown != () { out.ocf = (); out.implied_capex = (); }
    out
}

// Newest finished quarters in a row that reinvested less than `floor`; an unknown one ends it.
fn streak_of(quarters, floor) {
    let n = 0;
    for q in quarters {
        if q.reinvestment == () || q.reinvestment >= floor { break; }
        n += 1;
    }
    n
}

// Vehicles now minus GROWTH_MONTHS months ago, pure: () when either month is not in the store.
fn growth_of(months, date) {
    let now = mi_of(date);
    if now == () { return (); }
    // The newest month in the store at or before now, and the one GROWTH_MONTHS before it.
    let newest = ();
    for k in months.keys() { let m = mi_of(k); if m != () && m <= now && (newest == () || m > newest) { newest = m; } }
    if newest == () { return (); }
    let a = months[month_text(newest - global::GROWTH_MONTHS)];
    if type_of(a) != "map" { return (); }
    num(months[month_text(newest)].vehicles) - num(a.vehicles)
}

// The most valuable company other than `id` (openttd://companies), () when there is none.
fn leader_of(companies, id) {
    let best = ();
    for c in companies {
        if id_of(c.id) == id { continue; }
        if best == () || num(c.company_value) > num(best.company_value) { best = c; }
    }
    best
}

// The vehicles of a fleet {train, road, ship, aircraft} (openttd://companies), () without one.
fn fleet_of(fleet) {
    if type_of(fleet) != "map" { return (); }
    let n = 0;
    for k in ["train", "road", "ship", "aircraft"] { n += id_of(fleet[k] ?? 0); }
    n
}

// One company's measures, pure: its entry of openttd://companies, its economy, its months of the
// store (() when the store did not answer), the date, the thresholds.
fn measure(c, econ, months, date, thresholds) {
    let id = id_of(c.id);
    let have_months = type_of(months) == "map";
    let mo = if have_months { months } else { #{} };
    let qs = quarters_of(econ, mo, date);
    let year = trailing_of(qs);
    let net = num(c.money) - num(c.loan);
    let idle = ();
    let unknown = [];
    if year.income == () {
        unknown.push("idle ratio: the engine gave no income for a quarter");
    } else if year.income < global::MIN_INCOME.to_float() {
        unknown.push(`idle ratio: trailing-year income ${year.income.to_int()} is under ${global::MIN_INCOME}`);
    } else {
        idle = net / year.income;
    }
    if !have_months { unknown.push("store: capital_months did not answer"); }
    let growth = growth_of(mo, date);
    if growth == () { unknown.push(`vehicles growth: the store holds no month ${global::GROWTH_MONTHS} months back`); }
    for q in qs { if q.unknown != () { unknown.push(`${q.quarter}: ${q.unknown}`); } }
    #{ company: id, name: c.name, period: if mi_of(date) == () { () } else { month_text(mi_of(date)) }, game_date: date,
       writer: global::WRITER, version: global::VERSION,
       money: num(c.money), loan: num(c.loan), net: net, value: num(c.company_value),
       vehicles: fleet_of(c.fleet),
       quarters: qs, trailing: year, idle_ratio: idle, vehicles_growth: growth,
       streak: streak_of(qs, thresholds.reinvestment_floor.value),
       thresholds: thresholds, unknown: unknown }
}

// The leader's figures beside a company's, and the flags, pure: `m` the company's measures,
// `l` its leader's measures (() when it has none).
fn judged(m, l) {
    let out = m;
    out.leader = if l == () { () } else {
        #{ company: l.company, name: l.name, value: l.value,
           reinvestment: if l.quarters.is_empty() { () } else { l.quarters[0].reinvestment },
           trailing_reinvestment: l.trailing.reinvestment, idle_ratio: l.idle_ratio, vehicles_growth: l.vehicles_growth } };
    let flags = #{};
    let floor = m.thresholds.reinvestment_floor.value;
    if m.streak >= global::PERSIST {
        let rates = [];
        for i in 0..m.streak { rates.push(`${m.quarters[i].quarter} ${pct(m.quarters[i].reinvestment)} %`); }
        flags.under_reinvesting = `reinvested ${joined(rates)} of its operating cash flow, under the floor ${pct(floor)} % for ${m.streak} quarter(s)` +
            if out.leader != () && out.leader.reinvestment != () { `; ${out.leader.name} ${pct(out.leader.reinvestment)} % in ${m.quarters[0].quarter}` } else { "" };
    }
    let idle_at = m.thresholds.idle_years.value;
    if m.idle_ratio != () && m.idle_ratio >= idle_at && m.vehicles_growth != () {
        let lg = if out.leader == () { () } else { out.leader.vehicles_growth };
        if m.vehicles_growth <= 0.0 || (lg != () && m.vehicles_growth < lg) {
            flags.hoarding = `${m.net.to_int()} net in the bank is ${(m.idle_ratio * 10.0).round() / 10.0} year(s) of income (threshold ${idle_at}); vehicles ${m.vehicles_growth.to_int()} in ${global::GROWTH_MONTHS} months` +
                if lg != () { ` against ${out.leader.name}'s ${lg.to_int()}` } else { "" };
        }
    }
    out.flags = flags;
    out
}

// The log line for a company, () when there is nothing new: a flag holds and the newest finished
// quarter is one not said yet (so one line a quarter at most, and none again after a restart).
fn line_of(m, prev) {
    if m.flags.is_empty() || m.quarters.is_empty() { return (); }
    let q = m.quarters[0].quarter;
    if type_of(prev) == "map" && prev.said_quarter == q { return (); }
    let parts = [];
    for k in m.flags.keys() { parts.push(`${k}: ${m.flags[k]}`); }
    `capital_watch ${m.period} company ${m.company + 1} (${m.name}), ${q}: ${joined(parts)} [floor from ${m.thresholds.reinvestment_floor.source}, idle from ${m.thresholds.idle_years.source}]`
}

// One month: measure every company, compare it with its leader, write capital.<company>, print.
fn judge(date) {
    let companies = read("openttd://companies").data.companies ?? [];
    let answer = ();
    try { answer = api_dbNamedRun(#{ name: "capital_months", params: #{ months: global::MONTHS } }); }
    catch (err) { print(`capital_watch: store: ${err}`); }
    let store = by_company(answer);
    let store_rows = if type_of(answer) == "map" && store != () { (answer.rows ?? []).len() } else { () };
    let thresholds = #{
        reinvestment_floor: threshold_of(shared_get(global::DERIVED_FLOOR), global::DERIVED_FLOOR, global::REINVEST_FLOOR, "REINVEST_FLOOR", global::FLOOR_RANGE),
        idle_years: threshold_of(shared_get(global::DERIVED_IDLE), global::DERIVED_IDLE, global::IDLE_YEARS, "IDLE_YEARS", global::IDLE_RANGE) };
    let measured = #{};
    let getters = 0;
    for c in companies {
        let econ = economy(id_of(c.id));
        for e in econ { if e.income != () { getters += 1; } if e.expenses != () { getters += 1; } }
        let months = if store == () { () } else { store[`${id_of(c.id)}`] ?? #{} };
        measured[`${id_of(c.id)}`] = measure(c, econ, months, date, thresholds);
    }
    let reads = [
        #{ source: "openttd://game/state", rows: 1 },
        #{ source: "openttd://companies", rows: companies.len() },
        #{ source: "ScriptCompany.GetQuarterlyIncome/GetQuarterlyExpenses", quarters: global::QUARTERS, rows: getters },
        #{ source: "store:capital_months", params: #{ months: global::MONTHS }, rows: store_rows },
        #{ source: global::DERIVED_FLOOR, rows: if thresholds.reinvestment_floor.bootstrap { 0 } else { 1 } },
        #{ source: global::DERIVED_IDLE, rows: if thresholds.idle_years.bootstrap { 0 } else { 1 } } ];
    let out = [];
    for c in companies {
        let id = id_of(c.id);
        let l = leader_of(companies, id);
        let v = judged(measured[`${id}`], if l == () { () } else { measured[`${id_of(l.id)}`] });
        v.reads = reads;
        let key = `capital.${id}`;
        let prev = shared_get(key);
        let line = line_of(v, prev);
        v.said_quarter = if line != () { v.quarters[0].quarter } else if type_of(prev) == "map" { prev.said_quarter } else { () };
        if line != () { print(line); }
        shared_set(key, v);
        out.push(v);
    }
    print(`capital_watch ${date}: ${out.len()} company(ies) judged; flagged: ${out.filter(|v| !v.flags.is_empty()).map(|v| v.company + 1)}`);
    out
}

// --- the loop --------------------------------------------------------------------------------------
let looks = 0;
let judged_month = "";
print(`capital_watch: a look every ${ROUND_S} s; once a game month it measures every company's reinvestment and idle capital`);
loop {
    try {
        let date = `${read("openttd://game/state").game_date ?? ""}`;
        let mi = mi_of(date);
        if mi == () {
            print(`capital_watch: no game date yet ('${date}')`);
        } else if month_text(mi) != judged_month {
            judge(date);
            judged_month = month_text(mi);
        }
    } catch (err) {
        print(`capital_watch: ${err}`);
    }
    looks += 1;
    // One line a look, so the supervisor's stall limit above is not reached while the game is paused.
    print(`capital_watch look ${looks}: month ${if judged_month == "" { "not judged yet" } else { judged_month }}`);
    if rounds() > 0 && looks >= rounds() { break; }
    sleep(ROUND_S * 1000);
}
