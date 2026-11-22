// Live-tuner stage `brain` (RESEARCH-ONLY, research/battle-brain.md "Live tuner"): the battle
// brain's proposal. From ctx.brain (brain.json, schema battle-brain/v1, () when the runner has no
// loopback URL serving it) the first proposal of kind `kind` (else, with `any_kind`, the first
// one) proposes its settings; with `need_data` a brain without enough data is skipped. `hold` and
// `hold_for` as in the ga stage. Pure: no host call.
//
// params: kind, any_kind, need_data, hold, hold_for [stage], growth (from common).
import "policy/lib" as lib;

fn propose(ctx, params) {
    let me = "brain";
    let b = ctx.brain;
    if b == () { return [lib::note(me, "no brain.json this round ($TUNE_BRAIN_URL names no loopback URL serving it)")]; }
    if b.schema != "battle-brain/v1" { return [lib::note(me, `not a battle-brain/v1 record (${b.schema})`)]; }
    if params.need_data && b.enough_data == false { return [lib::note(me, "the brain has not enough data yet")]; }
    let ps = b.proposals ?? [];
    let pick = ();
    for p in ps { if p.kind == params.kind { pick = p; break; } }
    if pick == () && params.any_kind && !ps.is_empty() { pick = ps[0]; }
    if pick == () { return [lib::note(me, `no ${params.kind} proposal`)]; }
    let why = `brain ${pick.kind ?? "?"} proposal${if b.enough_data == false { " (thin data)" } else { "" }}`;
    let hold = params.hold && lib::active(ctx, params.hold_for);
    lib::from_settings(ctx, lib::parse_settings(pick.settings ?? ""), why, me, params.growth, hold)
}
