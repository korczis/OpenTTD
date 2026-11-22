//! RESEARCH-ONLY: one author per company for the freight-line slots (fr_*, fr2_* .. fr16_*;
//! research/battlebrain/README.md "One author per slot").
//!
//! The shared key `planner.<company>` (src/rhai_shared.rs) names the one writer that may propose a
//! company's freight-line slots; the cutover runbook sets it to `battlebrain` before a live pull. A
//! writer that finds another name there proposes nothing for those slots and says why. An unset
//! key (or a null value) leaves every writer as it was. This is the one Rust copy of the key's
//! format and of the rule; the Rhai copy is tools/research/mcp-panel/rhai/fr_author.rhai.
//!
//! Pure, so both the panel (through rhai_shared.rs, which reads its own memory) and mcp-intent
//! (through intent.rs, which is handed the value the panel's /api/rhai/shared answered) use it.

use serde_json::{json, Value};

/// The prefix every planner key starts with.
pub const PLANNER_KEY_PREFIX: &str = "planner.";

/// The shared key that names who plans `company`'s freight-line slots.
pub fn planner_key(company: i64) -> String {
    format!("{PLANNER_KEY_PREFIX}{company}")
}

/// The company a planner key is for, or None when `key` is not one.
pub fn company_of_key(key: &str) -> Option<i64> {
    key.strip_prefix(PLANNER_KEY_PREFIX)?.parse().ok()
}

/// Whether `writer` may propose `company`'s freight-line slots, given the value of the company's
/// planner key (`holder`, None when the key is absent). None: it may (the key is unset, null, or
/// names this writer). Otherwise the refusal, as data: `{key, company, holder, writer, why}`.
pub fn refusal(holder: Option<&Value>, company: i64, writer: &str) -> Option<Value> {
    let holder = holder.filter(|h| !h.is_null())?;
    let name = holder.as_str().map(str::to_string).unwrap_or_else(|| holder.to_string());
    if name == writer {
        return None;
    }
    let key = planner_key(company);
    Some(json!({"key": key, "company": company, "holder": holder, "writer": writer,
                "why": format!("company {company}'s freight-line slots are planned by {name} (shared key {key}), not by {writer}")}))
}

/// The refusal code of a change of a freight-line slot that names no writer, so its planner key
/// cannot be checked: a POST /api/bots/setting without `writer`, or a raw MCP call through the panel.
pub const NO_WRITER: &str = "fr_needs_writer";
/// The refusal code of a change of a freight-line slot whose writer is not the company's planner.
pub const NOT_THE_PLANNER: &str = "fr_planner_held";

/// Whether a request that changes one of `company`'s freight-line slots may be sent on behalf of
/// `writer` (None, or blank, when it names none). None: it names a writer and `refusal` lets it.
/// Otherwise the refusal as data, `code` NO_WRITER or NOT_THE_PLANNER: a request that names no
/// writer is refused whatever the key holds, since the key could not be checked against it.
pub fn change_refusal(holder: Option<&Value>, company: i64, writer: Option<&str>) -> Option<Value> {
    let Some(writer) = writer.map(str::trim).filter(|w| !w.is_empty()) else {
        let key = planner_key(company);
        return Some(json!({"code": NO_WRITER, "key": key, "company": company, "holder": holder.cloned().unwrap_or(Value::Null),
            "writer": Value::Null, "why": format!("a change of company {company}'s freight-line slots names its writer, \
                so that the shared key {key} can be checked against it; this request names none")}));
    };
    let mut r = refusal(holder, company, writer)?;
    r["code"] = json!(NOT_THE_PLANNER);
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_or_own_key_lets_the_writer_propose_and_another_name_refuses_it() {
        assert_eq!(planner_key(3), "planner.3");
        assert_eq!(company_of_key("planner.3"), Some(3));
        assert_eq!(company_of_key("planner.x"), None);
        assert_eq!(company_of_key("replan.3"), None);
        assert!(refusal(None, 0, "intent").is_none(), "unset: as before");
        assert!(refusal(Some(&Value::Null), 0, "intent").is_none(), "null: as unset");
        assert!(refusal(Some(&json!("intent")), 0, "intent").is_none(), "its own name");
        let r = refusal(Some(&json!("battlebrain")), 2, "intent").expect("another writer holds it");
        assert_eq!(r["key"], "planner.2");
        assert_eq!(r["holder"], "battlebrain");
        assert_eq!(r["writer"], "intent");
        assert!(r["why"].as_str().unwrap().contains("battlebrain"));
        // A value that is not a name is still another holder, never silently "unset".
        assert!(refusal(Some(&json!({"by": "battlebrain"})), 2, "intent").is_some());
    }

    /// A change that names no writer is refused whatever the key holds, even unset; one that names
    /// a writer is refused exactly when `refusal` refuses it, and the code says which.
    #[test]
    fn a_change_names_its_writer_and_that_writer_holds_the_key_or_no_one_does() {
        for holder in [None, Some(Value::Null), Some(json!("battlebrain"))] {
            for writer in [None, Some(""), Some("  ")] {
                let r = change_refusal(holder.as_ref(), 3, writer).expect("no writer: refused");
                assert_eq!((r["code"].as_str(), r["key"].as_str(), r["writer"].clone()), (Some(NO_WRITER), Some("planner.3"), Value::Null));
                assert_eq!(r["holder"], holder.clone().unwrap_or(Value::Null));
                assert!(r["why"].as_str().unwrap().contains("names none"), "{}", r["why"]);
            }
        }
        assert!(change_refusal(None, 3, Some("owner")).is_none(), "unset: any named writer");
        assert!(change_refusal(Some(&Value::Null), 3, Some("owner")).is_none(), "null: as unset");
        assert!(change_refusal(Some(&json!("battlebrain")), 3, Some(" battlebrain ")).is_none(), "the holder itself");
        let r = change_refusal(Some(&json!("battlebrain")), 3, Some("owner")).expect("another writer holds it");
        assert_eq!((r["code"].as_str(), r["holder"].as_str(), r["writer"].as_str()), (Some(NOT_THE_PLANNER), Some("battlebrain"), Some("owner")));
        assert_eq!(r["why"], refusal(Some(&json!("battlebrain")), 3, "owner").unwrap()["why"], "the same words as refusal");
    }
}
