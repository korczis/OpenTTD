-- The store describing itself: every table, view and column with its comment. Start here: SELECT * FROM schema_doc.
CREATE OR REPLACE VIEW schema_doc AS
  SELECT 'table' AS kind, table_name AS name, NULL AS column_name, NULL AS type, comment
  FROM duckdb_tables() WHERE schema_name = 'main'
  UNION ALL SELECT 'view', view_name, NULL, NULL, comment
  FROM duckdb_views() WHERE schema_name = 'main' AND NOT internal
  UNION ALL SELECT 'column', table_name, column_name, data_type, comment
  FROM duckdb_columns() WHERE schema_name = 'main' AND NOT internal;

COMMENT ON VIEW schema_doc IS 'The store describing itself: every table, view and column with its comment. Start here: SELECT * FROM schema_doc.';
