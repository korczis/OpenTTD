// PrismaticAI's autopilot floors, in one place (RESEARCH-ONLY). Import with:
//   import "bot_floors" as bf;
//
// The bot grows on its own while nobody gave it a plan. main.nut's `if (b.autopiloted)` block then
// raises own_lines and max_routes, caps cash_reserve, and sets intercity and self_steward. A plan
// is one complete freight slot, the rail pair or the air pair (main.nut, `b.planned`). Once the bot
// has one, the block no longer runs, and the bot's live values stand as they are. In run 12609 and
// in game 10487, a re-plan made the bot planned while its live own_lines was 4. The floor of 10
// went, and the bot built 4 lines of its own from then on: 1952-04-15 to 2114 in game 10487
// (research/mechanisms/planner-and-algoglyph.md "Replanning while the game runs").
//
// This library is the panel's one copy of those floors. guard() keeps a proposal from dropping
// them. The Rust test `the_floors_are_main_nuts_autopilot_block` (src/rhai_shared.rs) reads
// main.nut and fails when the block and FLOORS drift apart. Should the bot apply the floors planned
// or not (`if (b.grows)`), the guard costs nothing: the values it files are the ones the bot holds.
// planned() can only err towards the floors: the bot counts a slot only when all three are set,
// and also checks it and waits for two reads that agree (main.nut FreightLines).
//
// It names no setting of a line (rule project.decision-authority-boundaries B2). The groups of a
// line come from the names the bot declares: x_from/x_to/x_cargo is a freight line, x_a/x_b a
// pair. Nothing runs at import time.

// The autopilot floors, as main.nut's `if (b.autopiloted)` block applies them: `at_least` is
// max(value, n), `at_most` is min(value, n), `equals` is an assignment.
const FLOORS = #{
    self_steward: #{ equals: 1 },
    own_lines: #{ at_least: 10 },
    max_routes: #{ at_least: 30 },
    intercity: #{ equals: 1 },
    cash_reserve: #{ at_most: 5000 },
};
// The setting that turns the autopilot on (1) or off (0), and its default in info.nut.
const AUTOPILOT = "autopilot";
const AUTOPILOT_DEFAULT = 1;

/// The floors, the autopilot setting's name and its default, for a script or a test that imports
/// this library (a module's constants are not exported).
fn floors() { #{ floors: global::FLOORS, name: global::AUTOPILOT, "default": global::AUTOPILOT_DEFAULT } }

/// `value` held to `floor`: () when it already meets it, else the value it must have.
fn floor_value(floor, value) {
    if "equals" in floor { return if value == floor.equals { () } else { floor.equals }; }
    if "at_least" in floor && value < floor.at_least { return floor.at_least; }
    if "at_most" in floor && value > floor.at_most { return floor.at_most; }
    ()
}

/// The groups of settings that make one line, from the names the bot declares: every prefix with
/// `from`, `to` and `cargo` (a freight line, built by rail), and every prefix with `a` and `b` (a
/// pair: rail_ or air_). Each is #{ prefix, members, mode }.
fn groups(declared) {
    let out = [];
    for n in declared {
        if n.ends_with("_from") {
            let p = n.sub_string(0, n.len() - 4);
            if declared.contains(`${p}to`) && declared.contains(`${p}cargo`) {
                out.push(#{ prefix: p, members: [`${p}from`, `${p}to`, `${p}cargo`], mode: "rail" });
            }
        } else if n.ends_with("_a") {
            let p = n.sub_string(0, n.len() - 1);
            if declared.contains(`${p}b`) {
                out.push(#{ prefix: p, members: [`${p}a`, `${p}b`], mode: p.sub_string(0, p.len() - 1) });
            }
        }
    }
    out
}

/// Whether every member of `g` is set (>= 0) in `settings` (name -> value).
fn complete(g, settings) {
    for m in g.members {
        let v = settings[m];
        if type_of(v) == "()" || v < 0 { return false; }
    }
    true
}

/// Whether the bot runs on a plan with `settings`: one line group complete (main.nut `b.planned`).
fn planned(settings, gs) {
    for g in gs { if complete(g, settings) { return true; } }
    false
}

/// A proposal `diff` (name -> value) for a bot whose live settings are `live` (the [#{name, value}]
/// list of /api/bots), guarded so that it cannot drop the autopilot floors. Returns
/// #{ settings: the guarded diff, grouped: [names added so that a line's group is filed whole],
///    raised: [#{name, from, to}], lost: [#{name, from, to}: floors the live bot already runs
///    without, being planned with the autopilot on], planned_before, planned_after, autopilot,
///    added_modes: [modes of the line groups the plan completes or changes] }.
///   0. A proposal never turns the autopilot off: with it off there are no floors at all.
///   1. A group the proposal touches is filed whole: a slot completed from rows that the owner
///      approved months apart (12609: #227, #460, #461 made fr10 over nine months) cannot happen
///      within one plan.
///   2. With the autopilot on after the plan, a proposed floor key is held to its floor.
///   3. When the bot would be planned after the plan, a declared floor key whose live value the
///      block no longer lifts is proposed at its floor.
fn guard(live, diff) {
    let now = #{};
    let declared = [];
    for s in live { now[s.name] = s.value; declared.push(s.name); }
    let gs = groups(declared);
    let out = #{ settings: diff, grouped: [], raised: [], lost: [], planned_before: planned(now, gs),
                 planned_after: false, autopilot: 0, added_modes: [] };
    for g in gs {
        let touched = false;
        for m in g.members { if m in diff { touched = true; } }
        if !touched { continue; }
        for m in g.members {
            if !(m in out.settings) && m in now { out.settings[m] = now[m]; out.grouped.push(m); }
        }
    }
    let live_autopilot = now[global::AUTOPILOT] ?? global::AUTOPILOT_DEFAULT;
    if live_autopilot == 1 && global::AUTOPILOT in out.settings && out.settings[global::AUTOPILOT] != 1 {
        out.raised.push(#{ name: global::AUTOPILOT, from: out.settings[global::AUTOPILOT], to: 1 });
        out.settings.remove(global::AUTOPILOT);
    }
    let after = now;
    for k in out.settings.keys() { after[k] = out.settings[k]; }
    out.autopilot = after[global::AUTOPILOT] ?? global::AUTOPILOT_DEFAULT;
    out.planned_after = planned(after, gs);
    for g in gs {
        let touched = false;
        for m in g.members { if m in diff { touched = true; } }
        if touched && complete(g, after) && !out.added_modes.contains(g.mode) { out.added_modes.push(g.mode); }
    }
    if out.planned_before && live_autopilot == 1 {
        for k in global::FLOORS.keys() {
            if !declared.contains(k) { continue; }
            let v = floor_value(global::FLOORS[k], now[k]);
            if type_of(v) != "()" { out.lost.push(#{ name: k, from: now[k], to: v }); }
        }
    }
    if out.autopilot != 1 { return out; }
    for k in global::FLOORS.keys() {
        if !declared.contains(k) { continue; }
        let proposed = k in out.settings;
        if !proposed && !out.planned_after { continue; }
        let v = floor_value(global::FLOORS[k], after[k]);
        if type_of(v) == "()" { continue; }
        out.raised.push(#{ name: k, from: after[k], to: v });
        out.settings[k] = v;
    }
    out
}
