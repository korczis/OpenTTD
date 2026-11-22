// One author per freight-line slot (RESEARCH-ONLY; research/battlebrain/README.md "One author
// per slot"). Import with:  import "fr_author" as fa;
//
// The shared key planner.<company> names the one writer that may propose a company's
// freight-line slots (fr_*, fr2_* .. fr16_*). The cutover runbook sets it to "battlebrain" before
// a live pull (POST /api/rhai/shared {key: "planner.<id>", value: "battlebrain", rev: 0}). A
// script that finds another name there proposes nothing for those slots and says why; an unset
// key leaves it as it was. This is the one Rhai copy of the key's format and of the rule (the
// Rust one is tools/research/mcp-rhai/src/fr_author.rs): every script imports it, none spells
// the key itself. Nothing runs at import time.

/// The shared key that names who plans `company`'s freight-line slots.
fn planner_key(company) { `planner.${company}` }

/// Whether a setting is a freight-line slot of PrismaticAI: fr_ or fr<n>_ (the Rust source of
/// the slot names is battlebrain_contract::freight_slot_prefix; a slot the bot does not declare is
/// never proposed, so the open upper bound here costs nothing).
fn is_freight_slot(name) {
    let s = `${name}`;
    if !s.starts_with("fr") { return false; }
    for ch in s.sub_string(2).chars() {
        if ch == '_' { return true; }
        if !"0123456789".contains(ch) { return false; }
    }
    false
}

/// () when `writer` may propose `company`'s freight-line slots (its planner key is unset or names
/// `writer`), else why not, as data: #{key, company, holder, writer, why}.
fn refusal(company, writer) {
    let key = planner_key(company);
    let holder = shared_get(key);
    if type_of(holder) == "()" || `${holder}` == writer { return (); }
    #{ key: key, company: company, holder: holder, writer: writer,
       why: `company ${company}'s freight-line slots are planned by ${holder} (shared key ${key}), not by ${writer}` }
}

/// Whether a refusal (from refusal()) withholds the setting `name`.
fn withholds(r, name) { type_of(r) != "()" && is_freight_slot(name) }

/// `settings` (name -> value) as `writer` may propose them for `company`:
/// #{settings: the ones it may send, withheld: [names held back], refusal: () or why}.
fn withhold(company, writer, settings) {
    let r = refusal(company, writer);
    let out = #{ settings: settings, withheld: [], refusal: r };
    if type_of(r) == "()" { return out; }
    let keep = #{};
    for name in settings.keys() {
        if is_freight_slot(name) { out.withheld.push(name); } else { keep[name] = settings[name]; }
    }
    out.settings = keep;
    out
}
