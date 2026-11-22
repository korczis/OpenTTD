# The store's SQL

RESEARCH-ONLY. Every statement the DuckDB store runs on its own lives here, one file each, and
`build.rs` embeds whatever this directory holds. No Rust or Rhai code builds SQL by joining strings.

| Directory | What a file holds | When it runs |
|---|---|---|
| `tables/<name>.sql` | `CREATE TABLE IF NOT EXISTS <name>`, then `COMMENT ON TABLE` and one `COMMENT ON COLUMN` per column | at every open of a store, in name order |
| `views/NN_<name>.sql` | `CREATE OR REPLACE VIEW <name>`, then `COMMENT ON VIEW`. `NN` orders a view after the views it reads | at every open, after the tables, in `NN` order |
| `queries/<name>.sql` | one read-only statement with named parameters (`$company`), its leading `--` lines its description | on request: `POST /api/db/named {name, params}` |

Conventions:

- A file's name is the object's name: `views/30_company_latest.sql` creates `company_latest`, and
  a test fails when the two differ, or when a table, view or column has no comment.
- A query's parameters are read from the statement itself (DuckDB's prepared statement), never
  declared twice. `GET /api/db/named` lists every query with its description and parameters.
  Rhai reaches both as `api_dbNamed()` and `api_dbNamedRun(#{name, params})`.
- Values are bound, never pasted into the text: a parameter is a typed value of the prepared
  statement.
- The store describes itself from these comments: `SELECT * FROM schema_doc`.

See `research/mechanisms/event-bus-and-store.md`.
