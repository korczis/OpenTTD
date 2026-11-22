// One author per freight-line slot (RESEARCH-ONLY; research/battlebrain/README.md "One author
// per slot"). Import with:  import "fr_author" as fa;
//
// The shared key planner.<company> names the one writer that may propose a company's
// freight-line slots. The cutover runbook sets it to "battlebrain" before a live pull
// (POST /api/rhai/shared {key: "planner.<id>", value: "battlebrain", rev: 0}). A script that finds
// another name there proposes NO setting for that company, the freight-line slots among them, and
// says why: one writer per company, as the replan service already did. This library names no
// slot (rule project.decision-authority-boundaries B2), so it cannot hold back less than the
// whole proposal. An unset key leaves every script as it was. This is the one Rhai copy of the
// key's format and of the rule (the Rust one is tools/research/mcp-rhai/src/fr_author.rs): every
// script imports it, none spells the key itself. Nothing runs at import time.

/// The shared key that names who plans `company`'s freight-line slots.
fn planner_key(company) { `planner.${company}` }

/// () when `writer` may propose settings for `company` (its planner key is unset or names
/// `writer`), else why not, as data: #{key, company, holder, writer, why}.
fn refusal(company, writer) {
    let key = planner_key(company);
    let holder = shared_get(key);
    if type_of(holder) == "()" || `${holder}` == writer { return (); }
    #{ key: key, company: company, holder: holder, writer: writer,
       why: `company ${company}'s freight-line slots are planned by ${holder} (shared key ${key}), not by ${writer}` }
}

/// Whether a refusal (from refusal()) holds back what a writer proposes.
fn holds(r) { type_of(r) != "()" }

/// `settings` (name -> value) as `writer` may propose them for `company`:
/// #{settings: the ones it may send, withheld: [names held back], refusal: () or why}.
fn withhold(company, writer, settings) {
    let r = refusal(company, writer);
    let out = #{ settings: settings, withheld: [], refusal: r };
    if !holds(r) { return out; }
    out.withheld = settings.keys();
    out.settings = #{};
    out
}
