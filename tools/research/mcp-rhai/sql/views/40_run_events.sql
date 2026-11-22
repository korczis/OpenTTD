-- Every event with the run it belongs to (run_ms, seed) beside its own columns.
CREATE OR REPLACE VIEW run_events AS
  SELECT r.run_ms, r.seed, e.* FROM events e
  JOIN run_spans r ON e.instance = r.instance AND e.ts_ms >= r.run_ms AND e.ts_ms < r.until_ms;

COMMENT ON VIEW run_events IS 'Every event with the run it belongs to (run_ms, seed) beside its own columns.';
