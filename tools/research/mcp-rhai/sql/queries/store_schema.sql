-- Every table and view of the store with its columns and their comments, as GET /api/db/tables
-- lists them: one row per column.
SELECT CASE WHEN v.view_name IS NULL THEN 'table' ELSE 'view' END AS kind, c.table_name AS name,
       coalesce(t.comment, v.comment) AS comment, c.column_name, c.data_type, c.comment AS column_comment
FROM duckdb_columns() c
LEFT JOIN duckdb_tables() t ON t.schema_name = c.schema_name AND t.table_name = c.table_name
LEFT JOIN duckdb_views() v ON v.schema_name = c.schema_name AND v.view_name = c.table_name
WHERE c.schema_name = 'main' AND NOT c.internal
ORDER BY kind, name, c.column_index;
