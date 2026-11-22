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
}
