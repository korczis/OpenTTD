// Live tuner (RESEARCH-ONLY, research/battle-brain.md "Live tuner"): keeps the owner's PrismaticAI
// growing with the game. It is the runner of a chain of policy stages (rhai/policy/<stage>.rhai,
// each a pure `propose(ctx, params)` returning proposals #{name, value, why, stage}); the chain,
// its order, merge rule and every threshold are data (policy/chain.rhai), overridden each round
// by the Rhai shared-store key `policy.chain`, so a GA or the owner drives it without a file edit.
//
// Once a round (every_s, 600 s by default) the runner:
//   1. reads the chain: policy/chain.rhai's chain(), the shared key `policy.chain` merged over
//      it, env TUNE_EVERY_S / TUNE_MAX_REQUESTS / TUNE_IDLE_BUDGETS over both;
//   2. builds ctx once from host reads: the owner's company ($TUNE_COMPANY, else the first running
//      company of the panel's AI), every company and its standing against the leader, the running
//      bot's in-game settings with their value, min, max and step (api_bots(): every bound is the
//      running bot's own), the pending approvals, the GA's /api/tuning, brain.json
//      ($TUNE_BRAIN_URL, loopback), the engines the company runs and may buy, the rail types
//      open to it, and the game date;
//   3. runs the stages in order, each seeing the earlier ones' proposals in ctx.proposed;
//   4. merges them ("first" or "priority"; a veto drops a setting), drops a setting the running
//      bot does not declare (logged), fits each value to the setting's bounds and step, skips one
//      already at its value or with a request pending for the company (no stacking), and sends at
//      most max_requests through ttd::set_bot, the approval-gated openttd.ai.set_setting.
// Each change waits for the owner's Approve in the panel (or auto-approve when the owner switches
// it on), and the chat says so. The tuner never approves. $TUNE_DRY=1 logs and sends nothing.
//
// Hot reload: the panel restarts this service when this file or a top-level library it imports
// changes. policy/ files are imported by a path with a `/`, which the panel's watcher does not
// stamp today (panel_scripts.rs path_of), and the host's module resolver caches a module per run,
// so an edit under policy/ takes effect at the next start of the service; the shared key
// `policy.chain` takes effect at the next round. Run it on the Scripts page
// (/scripts/prismatic_tune), or POST /api/services/start {"name":"prismatic_tune"}.
import "ttd" as ttd;
import "policy/lib" as lib;
import "policy/chain" as defaults;
import "fr_author" as fa;

// The shared-store key whose value overrides the default chain each round.
const CHAIN_KEY = "policy.chain";
// This tuner's name for the planner key planner.<company> (fr_author.rhai): while the key names
// another writer (Battlebrain during a cutover), it asks for nothing for that company, so no
// freight-line slot either, whatever setting a stage or the GA's /api/tuning names. Every request
// names it, so the panel checks the same key before it asks the game (POST /api/bots/setting).
const WRITER = "prismatic_tune";

fn dry() { env("TUNE_DRY") == "1" }

fn say(text) {
    text.replace("\"", "'");
    con_say(`"[tune] ${text}"`);
}

// ------------------------------------------------------------------------------- the chain (pure)

// `over` (the shared key's value, () when absent) merged over `base`: stages, merge,
// max_requests and every_s replace, sources and every stage's params merge key by key.
fn chain_of(base, over) {
    if over == () || type_of(over) != "map" { return base; }
    let c = base;
    for k in ["stages", "merge", "max_requests", "every_s"] { if over[k] != () { c[k] = over[k]; } }
    if type_of(over.sources) == "map" { for k in over.sources.keys() { c.sources[k] = over.sources[k]; } }
    if type_of(over.params) == "map" {
        for s in over.params.keys() {
            let p = c.params[s] ?? #{};
            let o = over.params[s];
            if type_of(o) != "map" { continue; }
            for k in o.keys() { p[k] = o[k]; }
            c.params[s] = p;
        }
    }
    c
}

// The environment over the chain (env wins): TUNE_EVERY_S, TUNE_MAX_REQUESTS, TUNE_IDLE_BUDGETS.
fn with_env(c, every, most, idle) {
    if every != "" { c.every_s = parse_int(every); }
    if most != "" { c.max_requests = parse_int(most); }
    if idle != "" { let p = c.params.common ?? #{}; p.idle_budgets = parse_int(idle); c.params.common = p; }
    c.every_s = lib::as_int(c.every_s);
    c.max_requests = lib::as_int(c.max_requests);
    c
}

// A stage's params: common under its own.
fn stage_params(c, name) {
    let p = c.params.common ?? #{};
    let own = c.params[name] ?? #{};
    for k in own.keys() { p[k] = own[k]; }
    p
}

// A stage name names a file of policy/ only: letters, digits and underscores.
fn stage_name_ok(name) {
    if type_of(name) != "string" || name == "" || name == "lib" || name == "chain" { return false; }
    for ch in name.chars() { if !(ch == '_' || (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9')) { return false; } }
    true
}

// One stage: policy/<name>.rhai's propose(ctx, params), resolved by name, so a stage file added to
// policy/ is usable from the chain without a change here.
fn run_stage(name, ctx, params) {
    import `policy/${name}` as stage;
    stage::propose(ctx, params)
}

// Every stage of the chain in order: #{proposals: [...], notes: [text]}. A stage that fails or
// answers no array is noted and the chain goes on; a stage cannot sign as another one.
fn run_chain(c, ctx) {
    let out = #{ proposals: [], notes: [] };
    ctx.proposed = [];
    if type_of(c.stages) != "array" { out.notes.push(`stages: ${c.stages} is not an array, no stage run`); return out; }
    for name in c.stages {
        if !stage_name_ok(name) { out.notes.push(`${name}: not a stage name (letters, digits, _)`); continue; }
        let r = ();
        try { r = run_stage(name, ctx, stage_params(c, name)); }
        catch (err) { out.notes.push(`${name}: stage failed: ${err}`); continue; }
        if type_of(r) != "array" { out.notes.push(`${name}: answered ${type_of(r)}, not an array`); continue; }
        for p in r {
            if type_of(p) != "map" { continue; }
            p.stage = name;
            if p.note != () { out.notes.push(`${name}: ${p.note}`); continue; }
            if p.name == () { continue; }
            out.proposals.push(p);
            ctx.proposed.push(p);
        }
    }
    out
}

// The merge rule over every proposal: one per setting, in the order the settings were first
// proposed. "first": the earliest stage wins; "priority": the highest params.<stage>.priority, a
// tie to the later stage. A veto from any stage drops the setting.
fn merge_proposals(c, proposals, notes) {
    let rule = if c.merge == "first" { "first" } else { "priority" };
    if c.merge != rule { notes.push(`merge ${c.merge}: not first|priority, priority used`); }
    let best = #{};
    let order = [];
    let vetoed = #{};
    for p in proposals {
        if p.veto == true { vetoed[p.name] = p; continue; }
        let prio = lib::as_float(stage_params(c, p.stage).priority ?? 0);
        let b = best[p.name];
        if b == () { best[p.name] = #{ p: p, prio: prio }; order.push(p.name); }
        else if rule == "priority" && prio >= b.prio { best[p.name] = #{ p: p, prio: prio }; }
    }
    let out = [];
    for name in order {
        if name in vetoed { notes.push(`${name}: vetoed by ${vetoed[name].stage} (${vetoed[name].why})`); continue; }
        out.push(best[name].p);
    }
    #{ chosen: out, notes: notes }
}

// The merged proposals as requests: #{changes: [{name, from, to, why, stage}], notes}. A setting
// the running bot does not declare as in-game is dropped and logged; a value is fitted to its
// bounds and step; one at its value, or with a request pending, is not asked.
fn requests(ctx, chosen, notes) {
    let changes = [];
    for p in chosen {
        let s = ctx.settings[p.name];
        if s == () { notes.push(`${p.name} (${p.stage}): dropped, the running bot declares no in-game setting of that name`); continue; }
        if p.value == () { continue; }
        let to = lib::fit(s, p.value);
        if to == s.value { continue; }
        if p.name in ctx.pending { notes.push(`${p.name} -> ${to} (${p.stage}): a request waits already (#${ctx.pending[p.name]})`); continue; }
        changes.push(#{ name: p.name, from: s.value, to: to, why: p.why, stage: p.stage });
    }
    #{ changes: changes, notes: notes }
}

// What one round proposes from the chain and ctx, without a host call (a stage's import aside):
// #{changes, notes}.
fn plan(c, ctx) {
    let r = run_chain(c, ctx);
    let m = merge_proposals(c, r.proposals, r.notes);
    requests(ctx, m.chosen, m.notes)
}

// A round's #{changes, notes} as this tuner may ask for them: with a refusal (fa::refusal, ()
// when none) nothing, each change dropped named in notes with why.
fn own_changes(p, held) {
    let keep = [];
    for x in p.changes {
        if fa::holds(held) { p.notes.push(`${x.name} -> ${x.to} (${x.stage}): not asked, ${held.why}`); continue; }
        keep.push(x);
    }
    p.changes = keep;
    p
}

// ------------------------------------------------------------------------------ the ctx (pure)

// The settings a running bot may change in game, by name: {value, min, max, step}.
fn live_settings(bot) {
    let out = #{};
    for s in bot.settings ?? [] {
        if s.ingame == false || s.editable == false { continue; }
        out[s.name] = #{ value: lib::as_int(s.value), min: lib::as_int(s.min), max: lib::as_int(s.max),
                         step: if s.step == () || lib::as_int(s.step) < 1 { 1 } else { lib::as_int(s.step) } };
    }
    out
}

// Where the company stands against the leader (the other company of the highest value), from
// openttd://companies rows: {value, vehicles, money, leader, leader_value, leader_vehicles, gap},
// gap the larger of the value and vehicle ratios (1.0 when it leads), or () without the company.
fn standing(rows, company) {
    let me = ();
    let lead = ();
    for c in rows {
        if lib::as_int(c.id) == company { me = c; continue; }
        if lead == () || lib::as_int(c.company_value) > lib::as_int(lead.company_value) { lead = c; }
    }
    if me == () { return (); }
    let out = #{ value: lib::as_int(me.company_value), vehicles: lib::as_int(me.fleet?.total ?? 0),
                 money: lib::as_int(me.money), leader: (), leader_value: 0, leader_vehicles: 0, gap: 1.0 };
    if lead != () {
        out.leader = if (lead.custom_name ?? "") == "" { lead.name } else { lead.custom_name };
        out.leader_value = lib::as_int(lead.company_value);
        out.leader_vehicles = lib::as_int(lead.fleet?.total ?? 0);
        let by_value = lib::as_float(out.leader_value) / lib::as_float(if out.value < 1 { 1 } else { out.value });
        let by_fleet = lib::as_float(out.leader_vehicles) / lib::as_float(if out.vehicles < 1 { 1 } else { out.vehicles });
        let g = if by_value > by_fleet { by_value } else { by_fleet };
        out.gap = (g * 10.0).round() / 10.0;
    }
    out
}

// The fleet against the market: per engine type the slowest and fastest engine the company runs
// and the fastest it may buy now; the rail types open to it, under its trains, and the fastest
// buildable locomotive of each. () without engine data.
fn tech_of(company, vehicles, engines, railtypes) {
    if engines == () || vehicles == () { return (); }
    let by_id = #{};
    for e in engines { by_id[`${e.id}`] = e; }
    let types = #{};
    let used_rt = [];
    for v in vehicles {
        if lib::as_int(v.owner) != company || v.engine_id == () { continue; }
        let e = by_id[`${v.engine_id}`];
        if e == () || e.wagon == true { continue; }
        let k = types[e.type] ?? #{ used_slowest: (), used_slowest_name: (), used_fastest: (), best: (), best_name: () };
        let sp = lib::as_int(e.max_speed_kmh);
        if k.used_slowest == () || sp < k.used_slowest { k.used_slowest = sp; k.used_slowest_name = e.name; }
        if k.used_fastest == () || sp > k.used_fastest { k.used_fastest = sp; }
        types[e.type] = k;
        if e.type == "rail" { for rt in e.railtypes ?? [] { if !used_rt.contains(rt) { used_rt.push(rt); } } }
    }
    let best_rt = #{};
    for e in engines {
        if e.wagon == true || !(e.buildable_by ?? []).contains(company) { continue; }
        let sp = lib::as_int(e.max_speed_kmh);
        let k = types[e.type];
        if k != () && (k.best == () || sp > k.best) { k.best = sp; k.best_name = e.name; types[e.type] = k; }
        if e.type == "rail" {
            for rt in e.railtypes ?? [] {
                let b = best_rt[rt];
                if b == () || sp > b.speed { best_rt[rt] = #{ speed: sp, name: e.name }; }
            }
        }
    }
    let open = [];
    for r in railtypes ?? [] { if (r.available_to ?? []).contains(company) { open.push(r.label); } }
    #{ types: types, railtypes: #{ available: open, used: used_rt, best_engine: best_rt } }
}

// The names of the company's pending set_setting requests: name -> approval id.
fn pending_of(st, company) {
    let out = #{};
    for r in st.pending ?? [] {
        let c = r.args?.company;
        if r.tool != "openttd.ai.set_setting" || c == () || lib::as_int(c) != company { continue; }
        out[r.args.name] = r.id;
    }
    out
}

// The owner's company: `fixed` ($TUNE_COMPANY) when set, else the first running company of the
// panel's AI; () while no such bot runs.
fn owner_of(r, fixed) {
    if fixed != "" { return parse_int(fixed); }
    let ai = r.status?.ai ?? "";
    for b in r.bots?.running ?? [] { if ai == "" || b.ai == ai { return lib::as_int(b.id); } }
    ()
}

// ctx from one round's reads (see reads()), or #{error} when there is nothing to tune.
fn build_ctx(r, fixed) {
    let company = owner_of(r, fixed);
    if company == () { return #{ error: "no running company of the panel's AI yet" }; }
    let bot = ();
    for b in r.bots?.running ?? [] { if lib::as_int(b.id) == company { bot = b; } }
    if bot == () { return #{ error: `company ${company} runs no AI` }; }
    #{ company: company, bot: bot.name, ai: bot.ai, date: r.date ?? "",
       settings: live_settings(bot), companies: r.companies ?? [],
       standing: standing(r.companies ?? [], company), pending: pending_of(r.status ?? #{}, company),
       tuning: r.tuning, brain: r.brain, tech: tech_of(company, r.vehicles, r.engines, r.railtypes),
       proposed: [] }
}

// ------------------------------------------------------------------------- host reads and sends

// The round's host reads: #{status, bots, companies, date, engines, railtypes, vehicles, tuning,
// brain, errors}; a read that fails leaves its field () and says why in errors.
fn reads(c) {
    let r = #{ errors: [] };
    r.status = status();
    r.bots = api_bots();
    let comp = read("openttd://companies");
    r.companies = comp.data.companies;
    r.date = comp.game_date;
    try {
        r.engines = read("openttd://engines").data.engines;
        r.railtypes = read("openttd://railtypes").data.types;
        let v = read("openttd://vehicles");
        r.vehicles = v.data.vehicles;
        if v.truncated == true { r.errors.push("openttd://vehicles truncated: the fleet is partly read"); }
    } catch (err) { r.errors.push(`engines/vehicles: ${err}`); }
    try {
        let t = loopback_get(c.sources.tuning);
        r.tuning = if t.enabled == false { () } else { t };
    } catch (err) { r.errors.push(`${c.sources.tuning}: ${err}`); }
    let url = env("TUNE_BRAIN_URL");
    if url != "" { try { r.brain = loopback_get(url); } catch (err) { r.errors.push(`brain ${url}: ${err}`); } }
    r
}

// The chain in force this round and where it came from: #{chain, source}.
fn chain_now() {
    let over = ();
    let source = "policy/chain.rhai";
    try {
        let e = shared_entry(global::CHAIN_KEY);
        if e != () { over = e.value; source += ` + shared ${global::CHAIN_KEY} rev ${e.rev} by ${e.writer}`; }
    } catch (err) { source += ` (shared ${global::CHAIN_KEY} unreadable: ${err})`; }
    let c = with_env(chain_of(defaults::chain(), over), env("TUNE_EVERY_S"), env("TUNE_MAX_REQUESTS"), env("TUNE_IDLE_BUDGETS"));
    #{ chain: c, source: source }
}

// One round; answers the seconds to the next one. Not named round(): a script function without
// parameters is also a method of every value in Rhai, so it would shadow the float's own round().
fn tune_round() {
    let now = chain_now();
    let c = now.chain;
    let r = reads(c);
    for e in r.errors { print(`tune: ${e}`); }
    let ctx = build_ctx(r, env("TUNE_COMPANY"));
    if ctx.error != () { print(`tune: ${ctx.error}`); return c.every_s; }
    let p = own_changes(plan(c, ctx), fa::refusal(ctx.company, global::WRITER));
    print(`tune: ${ctx.date} company ${ctx.company} (${ctx.bot}), chain ${c.stages} (${c.merge}) from ${now.source}, ${ctx.pending.len()} request(s) pending`);
    for n in p.notes { print(`tune:   ${n}`); }
    let sent = 0;
    for x in p.changes {
        if sent >= c.max_requests { print(`tune:   ${x.name} ${x.from} -> ${x.to}: next round (${c.max_requests} a round)`); continue; }
        if dry() { print(`tune:   ${x.name} ${x.from} -> ${x.to} (${x.why}), dry run`); sent += 1; continue; }
        try {
            let a = ttd::set_bot(ctx.company, x.name, x.to, global::WRITER);
            let id = a.approval_id ?? ();
            print(`tune:   ${x.name} ${x.from} -> ${x.to} (${x.why}): ${if id == () { `${a.status ?? a}` } else { `pending approval #${id}` }}`);
            say(`proposes ${x.name} ${x.from} -> ${x.to} (${x.why})`);
            sent += 1;
        } catch (err) {
            print(`tune:   ${x.name} not asked: ${err}`);
        }
    }
    if sent > 0 && !dry() {
        say(`${sent} change(s) wait for your Approve in the control panel (or switch on auto-approve there)`);
    }
    print(`tune: ${sent} request(s) this round, ${p.changes.len()} proposed`);
    c.every_s
}

// ---- main (the unit harness stops reading here)
let first = chain_now();
print(`prismatic_tune: chain ${first.chain.stages} (${first.chain.merge}) from ${first.source}, every ${first.chain.every_s}s, at most ${first.chain.max_requests} request(s) a round${if dry() { ", dry run" } else { "" }}`);
loop {
    let wait = first.chain.every_s;
    try { wait = tune_round(); } catch (err) { print(`tune: round failed: ${err}`); }
    sleep(lib::as_int(wait) * 1000);
}
