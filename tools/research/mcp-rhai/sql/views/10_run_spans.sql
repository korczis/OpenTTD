-- Every run of this store: instance, session, seed, run_ms (its start) and until_ms (the next run's start, or the end of time).
CREATE OR REPLACE VIEW run_spans AS
  SELECT instance, session, seed, started_ms AS run_ms,
  coalesce(lead(started_ms) OVER (PARTITION BY instance ORDER BY started_ms), 9223372036854775807) AS until_ms
  FROM runs;

COMMENT ON VIEW run_spans IS 'Every run of this store: instance, session, seed, run_ms (its start) and until_ms (the next run''s start, or the end of time).';
