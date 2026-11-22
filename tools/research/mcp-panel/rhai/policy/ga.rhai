// Live-tuner stage `ga` (RESEARCH-ONLY, research/battle-brain.md "Live tuner"): the genetic
// algorithm's best measured variant. From ctx.tuning (the LAN battle's /api/tuning, () when it is
// unreachable or off) the variant with the highest fitness over at least `min_matches` matches
// proposes its settings; with `genes_only` only the genes of the GA's space. With `hold`, while an
// earlier stage of `hold_for` proposed this round, a value that moves a knob against its growth
// direction (common.growth) is noted and not proposed. Pure: no host call.
//
// params: min_matches, genes_only, hold, hold_for [stage], growth (from common).
import "policy/lib" as lib;

fn propose(ctx, params) {
    let me = "ga";
    let t = ctx.tuning;
    if t == () { return [lib::note(me, "GA standings unreachable or off: skipped")]; }
    let top = ();
    for v in t.variants ?? [] {
        let f = v.fitness;
        if f != () && f.matches >= params.min_matches && (top == () || f.score > top.score) {
            top = #{ label: v.label, settings: v.settings, score: f.score, matches: f.matches };
        }
    }
    if top == () { return [lib::note(me, `no variant with ${params.min_matches} matches yet`)]; }
    let all = lib::parse_settings(top.settings);
    let genes = #{};
    for name in all.keys() {
        if params.genes_only && t.space != () && !(name in t.space) { continue; }
        genes[name] = all[name];
    }
    let hold = params.hold && lib::active(ctx, params.hold_for);
    lib::from_settings(ctx, genes, `GA best ${top.label} (score ${top.score}, ${top.matches} matches)`, me, params.growth, hold)
}
