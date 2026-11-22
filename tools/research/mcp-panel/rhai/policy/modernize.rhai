// Live-tuner stage `modernize` (RESEARCH-ONLY, research/battle-brain.md "Live tuner"): a fleet
// older than the market is turned over. From ctx.tech (the engines the company runs against the
// ones it may buy now, and the rail types open to it, read by the runner from openttd://vehicles,
// openttd://engines and openttd://railtypes): when, for some engine type, the best buildable
// engine is at least `speed_gain` times as fast as the slowest one the company runs, `engine`
// is set (modernize on); when a rail type open to the company and not under its trains carries a
// buildable locomotive at least `speed_gain` times as fast as its fastest train, `track` is set
// (upgrade_track on). Pure: no host call.
//
// params: speed_gain, engine #{name: value}, track #{name: value}.
import "policy/lib" as lib;

fn set_all(ctx, switches, why, me) {
    let out = [];
    for name in switches.keys() {
        let s = ctx.settings[name];
        if s == () { out.push(lib::proposal(name, switches[name], why, me)); continue; }
        let to = lib::fit(s, switches[name]);
        if to != s.value { out.push(lib::proposal(name, to, why, me)); }
    }
    out
}

fn propose(ctx, params) {
    let me = "modernize";
    let t = ctx.tech;
    if t == () { return [lib::note(me, "no engine data this round")]; }
    let gain = lib::as_float(params.speed_gain);
    let out = [];
    let behind = [];
    for kind in t.types.keys() {
        let k = t.types[kind];
        if k.used_slowest == () || k.best == () { continue; }
        if lib::as_float(k.best) >= lib::as_float(k.used_slowest) * gain {
            behind.push(`${kind} ${k.used_slowest_name} ${k.used_slowest} km/h < ${k.best_name} ${k.best} km/h`);
        }
    }
    if behind.is_empty() { out.push(lib::note(me, `the fleet runs the market's engines (gain ${gain})`)); }
    else { out += set_all(ctx, params.engine, `modernize: ${behind[0]}${if behind.len() > 1 { ` and ${behind.len() - 1} more` } else { "" }}`, me); }

    let rail = t.types.rail;
    let fastest = if rail == () || rail.used_fastest == () { 0.0 } else { lib::as_float(rail.used_fastest) };
    let better = ();
    for label in t.railtypes.available {
        if t.railtypes.used.contains(label) { continue; }
        let best = t.railtypes.best_engine[label];
        if best == () || fastest <= 0.0 { continue; }
        if lib::as_float(best.speed) >= fastest * gain && (better == () || best.speed > better.speed) {
            better = #{ label: label, speed: best.speed, name: best.name };
        }
    }
    if better == () { out.push(lib::note(me, `no open rail type beats the trains (${t.railtypes.used} used, ${t.railtypes.available} open)`)); }
    else { out += set_all(ctx, params.track, `modernize: rail type ${better.label} carries ${better.name} ${better.speed} km/h > ${fastest} km/h`, me); }
    out
}
