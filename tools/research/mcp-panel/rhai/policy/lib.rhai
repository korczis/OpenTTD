// Live-tuner policy helpers (RESEARCH-ONLY, research/battle-brain.md "Live tuner"): the pure pieces
// every stage of the chain shares. No host function is called here, so a stage that imports it
// stays testable offline. Not a stage itself (it has no propose()).
//
// A proposal is #{name, value, why, stage}; a veto is #{name, veto: true, why, stage} (it drops
// the name for the round, whatever proposed it); a note is #{stage, note} (logged, nothing set).

fn as_int(x) { if type_of(x) == "f64" { x.to_int() } else if x == () { 0 } else { x } }
fn as_float(x) { if type_of(x) == "f64" { x } else if x == () { 0.0 } else { x.to_float() } }
fn clamp(v, lo, hi) { if v < lo { lo } else if v > hi { hi } else { v } }

fn proposal(name, value, why, stage) { #{ name: name, value: value, why: why, stage: stage } }
fn veto(name, why, stage) { #{ name: name, veto: true, why: why, stage: stage } }
fn note(stage, text) { #{ stage: stage, note: text } }

// "a=1,b=2" -> #{a: 1, b: 2} (a battle settings string, as the GA and brain.json carry them).
fn parse_settings(s) {
    let m = #{};
    for kv in `${s}`.split(",") {
        let p = kv.split("=");
        // Rhai's trim changes the string in place and returns (), so it never chains.
        if p.len() == 2 { let k = p[0]; let v = p[1]; k.trim(); v.trim(); m[k] = parse_int(v); }
    }
    m
}

// `value` moved onto the setting's grid: clamped to [min, max] and snapped to min + k * step.
fn fit(s, value) {
    let v = clamp(as_int(value), s.min, s.max);
    let step = if s.step < 1 { 1 } else { s.step };
    clamp(s.min + ((v - s.min) + step / 2) / step * step, s.min, s.max)
}

// A move of `steps` of the setting's own step in direction `dir` (+1 / -1), on its grid.
fn stepped(s, dir, steps) { fit(s, s.value + dir * steps * s.step) }

// The company's idle money: what it holds above cash_reserve + `budgets` line budgets (both read
// from the running bot's own settings), with the floor and the budgets held.
fn idle(ctx, budgets) {
    let reserve = ctx.settings.cash_reserve?.value ?? 0;
    let budget = ctx.settings.line_budget?.value ?? 1;
    if budget < 1 { budget = 1; }
    let money = ctx.standing?.money ?? 0;
    let floor = reserve + budgets * budget;
    #{ money: money, floor: floor, idle: money - floor, budget: budget,
       held: (money - reserve) / budget }
}

// One step more for every `per` times `base` reached, at most `most` (1 at the first).
fn ladder(x, base, per, most) {
    let n = 1;
    let g = as_float(x) / as_float(if base == 0 { 1 } else { base });
    let p = as_float(if per <= 1 { 2 } else { per });
    while g >= p && n < most { g /= p; n += 1; }
    n
}

// The latest earlier proposal (not a note, not a veto) for `name` in ctx.proposed, or ().
fn earlier(ctx, name) {
    let out = ();
    for p in ctx.proposed ?? [] { if p.name == name && p.veto != true && p.note == () { out = p; } }
    out
}

// Whether an earlier stage named in `stages` proposed anything this round (its regime holds).
fn active(ctx, stages) {
    for p in ctx.proposed ?? [] { if p.note == () && p.veto != true && stages.contains(p.stage) { return true; } }
    false
}

// The settings map of a source ({name: value}) as proposals from `stage`, every value clamped to
// the bot's bounds; a name the bot does not declare passes through (the runner drops it and logs
// it); with `hold` on, a move against a growth direction (common.growth) is noted, not proposed.
fn from_settings(ctx, settings, why, stage, growth, hold) {
    let out = [];
    for name in settings.keys() {
        let s = ctx.settings[name];
        if s == () { out.push(proposal(name, settings[name], why, stage)); continue; }
        let to = fit(s, settings[name]);
        if to == s.value { continue; }
        let d = growth[name];
        if hold && d != () && (to - s.value) * d < 0 {
            out.push(note(stage, `${name}: ${why} wants ${to}, kept at ${s.value} while growth holds`));
            continue;
        }
        out.push(proposal(name, to, why, stage));
    }
    out
}
