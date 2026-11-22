// Live-tuner stage `rich` (RESEARCH-ONLY, research/battle-brain.md "Live tuner"): money that sits
// idle is growth not bought. While the company holds more than cash_reserve + idle_budgets line
// budgets (both the bot's own settings), every knob of `knobs` moves one step in its growth
// direction (common.growth), one more for every doubling of the idle money past `budgets_per_step`
// line budgets, at most `steps_max`; `switches` are set to the value given (rich_growth on). Pure: no host call.
//
// params: idle_budgets, budgets_per_step, steps_max, knobs [name], switches #{name: value},
//         growth #{name: +1|-1} (from common).
import "policy/lib" as lib;

fn propose(ctx, params) {
    let me = "rich";
    let out = [];
    let m = lib::idle(ctx, params.idle_budgets);
    if ctx.standing == () { return [lib::note(me, "the company is not in openttd://companies")]; }
    if m.idle <= 0 {
        return [lib::note(me, `money ${m.money}, idle above ${m.floor}: none`)];
    }
    let steps = lib::ladder(m.idle, params.budgets_per_step * m.budget, 2, params.steps_max);
    let why = `${m.idle} idle above ${m.floor} (${m.held} line budgets held), ${steps} step(s)`;
    out.push(lib::note(me, why));
    for name in params.switches.keys() {
        let s = ctx.settings[name];
        if s == () { out.push(lib::proposal(name, params.switches[name], `rich: ${why}`, me)); continue; }
        let to = lib::fit(s, params.switches[name]);
        if to != s.value { out.push(lib::proposal(name, to, `rich: ${why}`, me)); }
    }
    for name in params.knobs {
        let d = params.growth[name];
        if d == () { out.push(lib::note(me, `${name}: no growth direction in common.growth`)); continue; }
        let s = ctx.settings[name];
        if s == () { out.push(lib::proposal(name, (), `rich: ${why}`, me)); continue; }
        let to = lib::stepped(s, d, steps);
        if to != s.value { out.push(lib::proposal(name, to, `rich: ${why}`, me)); }
    }
    out
}
