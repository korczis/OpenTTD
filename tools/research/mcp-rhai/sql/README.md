# The store's SQL

RESEARCH-ONLY. Every statement this crate runs lives here, one file each, and `build.rs` embeds
whatever these directories hold (`src/store_sql.rs`). No Rust or Rhai code builds SQL by joining
strings: a statement runs as its file says, and every value is a named parameter (`$company`) the
database binds.

| Directory | What a file holds | When it runs |
|---|---|---|
| `tables/<name>.sql` | `CREATE TABLE IF NOT EXISTS <name>`, then `COMMENT ON TABLE` and one `COMMENT ON COLUMN` per column | at every open of a store, in name order |
| `views/NN_<name>.sql` | `CREATE OR REPLACE VIEW <name>`, then `COMMENT ON VIEW` and one `COMMENT ON COLUMN` per column. `NN` orders a view after the views it reads | at every open, after the tables, in `NN` order |
| `queries/<name>.sql` | one read-only statement with named parameters (`$company`), its leading `--` lines its description | on request: `POST /api/db/named {name, params}`, and the panel's own reads (`Named`) |
| `inserts/<table>.sql` | one row of `<table>`: `INSERT INTO <table> (<every column>) VALUES ($<column>, ...)` | by the store's writer (`Store::insert_*`), never on request |
| `postmortem/battle/<name>.sql` | one read-only statement on the LAN battle's SQLite store (`battle.sqlite`), `$edition` bound | by `battle-postmortem`, also when built without the `store` feature |
| `postmortem/analytics/<name>.sql` | `schema.sql` (its tables), and per table `delete_<table>.sql` and `insert_<table>.sql` | by `battle-postmortem` writing `analytics.duckdb` (feature `store`) |

Conventions:

- A file's name is the object's name: `views/30_company_latest.sql` creates `company_latest`, and
  a test fails when the two differ, or when a table, view or column has no comment.
- A view's columns are the ones DuckDB derives from its `SELECT` (`*` and joins included), and the
  test reads them from the store (`duckdb_columns()`), so a column a changed view gains needs its
  `COMMENT ON COLUMN` too. Write what the column means from the `SELECT` and the comment of the
  column it comes from. `CREATE OR REPLACE VIEW` drops the view's comments, so they follow it in
  the same file and are written again at every open.
- A query's parameters are read from the statement itself (DuckDB's prepared statement), never
  declared twice. `GET /api/db/named` lists every query with its description and parameters.
  Rhai reaches both as `api_dbNamed()` and `api_dbNamedRun(#{name, params})`.
- Values are bound, never pasted into the text: a parameter is a typed value of the prepared
  statement. A query whose shape would vary takes the variation as a value instead: an optional
  filter is `($seed IS NULL OR r.seed = $seed)` (`calibration_readings`), a count is `LIMIT $limit`.
- Only `queries/` and `postmortem/battle/` hold reads, and a test runs each through the read-only
  guard (`read_only_sql`); `inserts/` and `postmortem/analytics/` write, so a test checks instead
  that an insert names every column of its table, each from the parameter of its name, and that
  every analytics file writes a table `schema.sql` creates. Writes never reach `/api/db/named`,
  which serves `queries/` only.
- What a decision read is recorded by name: its `inputs.stored` is `{query, name, params, rows}`,
  the statement's text, its file and the values bound.
- The store describes itself from these comments: `SELECT * FROM schema_doc`.

See `research/mechanisms/event-bus-and-store.md`.
