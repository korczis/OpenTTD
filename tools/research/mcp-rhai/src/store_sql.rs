//! RESEARCH-ONLY. The SQL of this crate, one file each under `sql/` (`sql/README.md`), embedded by
//! build.rs from the directories, each `(name, text)` in file order:
//!
//! - the DuckDB store of every publisher (src/bus_sinks.rs): `TABLE_FILES`, `VIEW_FILES`,
//!   `QUERY_FILES` (read-only, on request) and `INSERT_FILES` (the writer's rows);
//! - battle-postmortem (research/battle-postmortem.md): `BATTLE_FILES` (the battle's SQLite store,
//!   read-only) and `ANALYTICS_FILES` (its analytics.duckdb).
//!
//! No code builds SQL from pieces: a statement runs as its file says, and every value is a named
//! parameter (`$company`) bound by the database.

use serde_json::{json, Map, Value};

include!(concat!(env!("OUT_DIR"), "/sql_files.rs"));

/// A file of `files` by name, or None.
pub fn find(files: &[(&str, &'static str)], name: &str) -> Option<&'static str> {
    files.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// A file of `files` by name, or an error that names the directory it is missing from.
pub fn file(files: &[(&str, &'static str)], dir: &str, name: &str) -> Result<&'static str, String> {
    find(files, name).ok_or_else(|| format!("no file sql/{dir}/{name}.sql"))
}

/// A named query's text, or None.
pub fn query(name: &str) -> Option<&'static str> {
    find(QUERY_FILES, name)
}

/// A file's description: its leading `--` lines, joined into one.
pub fn description(text: &str) -> String {
    text.lines().take_while(|l| l.trim_start().starts_with("--"))
        .map(|l| l.trim_start().trim_start_matches("--").trim()).collect::<Vec<_>>().join(" ")
}

/// A named query of `sql/queries` with the values of its parameters: what a reader asks a store
/// (POST /api/db/named takes `to_json`), and what a plan.* event records as its stored input.
#[derive(Clone, Debug, PartialEq)]
pub struct Named {
    pub name: &'static str,
    pub params: Map<String, Value>,
}

impl Named {
    /// `params` is an object of parameter values; anything else gives none.
    pub fn new(name: &'static str, params: Value) -> Named {
        Named { name, params: params.as_object().cloned().unwrap_or_default() }
    }

    /// The file's text, empty for a name no file has (a store then refuses it as no statement).
    pub fn sql(&self) -> &'static str {
        query(self.name).unwrap_or("")
    }

    /// `{name, params}`: the body of POST /api/db/named.
    pub fn to_json(&self) -> Value {
        json!({"name": self.name, "params": self.params})
    }

    /// `{query, name, params}`: the statement as it ran and the values bound to it, as an event or
    /// a record names what it read.
    pub fn record(&self) -> Value {
        json!({"query": self.sql(), "name": self.name, "params": self.params})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_query_carries_its_file_and_values() {
        let q = Named::new("company_rank", json!({"company": 1}));
        assert_eq!(q.sql(), query("company_rank").unwrap());
        assert!(q.sql().contains("$company"));
        assert_eq!(q.to_json(), json!({"name": "company_rank", "params": {"company": 1}}));
        assert_eq!(q.record(), json!({"query": q.sql(), "name": "company_rank", "params": {"company": 1}}));
        assert_eq!(Named::new("company_rank", json!([1])).params, Map::new(), "not an object: no values");
        assert_eq!(Named::new("no_such_query", json!({})).sql(), "");
        assert_eq!(file(INSERT_FILES, "inserts", "runs").map(|t| t.contains("INSERT INTO runs")), Ok(true));
        assert_eq!(file(INSERT_FILES, "inserts", "nope"), Err("no file sql/inserts/nope.sql".into()));
        assert_eq!(description("-- one\n  -- two\nSELECT 1 -- not this"), "one two");
    }
}
