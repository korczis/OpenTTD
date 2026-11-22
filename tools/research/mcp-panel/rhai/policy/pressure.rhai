// Live-tuner stage `pressure` (RESEARCH-ONLY, research/battle-brain.md "Live tuner"): the gap to
// the leader loosens the caps. While the company is more than `min_gap` times behind the leader
// (company value or vehicles, ctx.standing.gap) and, with `require_idle`, money sits idle, every
// knob of `knobs` moves in its growth direction by one step, one more for every `gap_per_step`
// times the gap grows, at most `steps_max`. It refines an earlier stage: where one already moved
// a knob further, that value is kept. `switches` (compete on) are set while behind. Pure.
//
// params: min_gap, gap_per_step, steps_max, require_idle, idle_budgets, knobs [name],
//         switches #{name: value}, growth #{name: +1|-1} (from common).
import "policy/lib" as lib;

fn propose(ctx, params) {
    let me = "pressure";
    let st = ctx.standing;
    if st == () { return [lib::note(me, "the company is not in openttd://companies")]; }
    if st.leader == () || st.gap <= lib::as_float(params.min_gap) {
        return [lib::note(me, `gap ${st.gap}x to ${st.leader ?? "nobody"} (at most ${params.min_gap}x): no pressure`)];
    }
    let m = lib::idle(ctx, params.idle_budgets);
    if params.require_idle && m.idle <= 0 {
        return [lib::note(me, `${st.gap}x behind ${st.leader} but no idle money above ${m.floor}`)];
    }
    let steps = lib::ladder(st.gap, 1, params.gap_per_step, params.steps_max);
    let why = `pressure: ${st.gap}x behind ${st.leader} (value ${st.value} vs ${st.leader_value}, vehicles ${st.vehicles} vs ${st.leader_vehicles}), ${steps} step(s)`;
    let out = [];
    for name in params.switches.keys() {
        let s = ctx.settings[name];
        if s == () { out.push(lib::proposal(name, params.switches[name], why, me)); continue; }
        let to = lib::fit(s, params.switches[name]);
        if to != s.value { out.push(lib::proposal(name, to, why, me)); }
    }
    for name in params.knobs {
        let d = params.growth[name];
        if d == () { out.push(lib::note(me, `${name}: no growth direction in common.growth`)); continue; }
        let s = ctx.settings[name];
        if s == () { out.push(lib::proposal(name, (), why, me)); continue; }
        let to = lib::stepped(s, d, steps);
        let e = lib::earlier(ctx, name);
        if e != () && e.value != () && (e.value - to) * d > 0 { to = e.value; }
        if to != s.value { out.push(lib::proposal(name, to, why, me)); }
    }
    out
}
