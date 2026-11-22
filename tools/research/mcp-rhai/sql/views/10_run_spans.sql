-- Every run of this store: instance, session, seed, run_ms (its start) and until_ms (the next run's start, or the end of time).
CREATE OR REPLACE VIEW run_spans AS
  SELECT instance, session, seed, started_ms AS run_ms,
  coalesce(lead(started_ms) OVER (PARTITION BY instance ORDER BY started_ms), 9223372036854775807) AS until_ms
  FROM runs;

COMMENT ON VIEW run_spans IS 'Every run of this store: instance, session, seed, run_ms (its start) and until_ms (the next run''s start, or the end of time).';
COMMENT ON COLUMN run_spans.instance IS 'the publisher of the run, e.g. panel-8800 or hub-8790 (runs.instance)';
COMMENT ON COLUMN run_spans.session IS 'the Claude or Majordomus session that started the run, or null (runs.session)';
COMMENT ON COLUMN run_spans.seed IS 'the run''s map seed, or null for a publisher without a game (runs.seed)';
COMMENT ON COLUMN run_spans.run_ms IS 'when the run started, Unix milliseconds (runs.started_ms); with instance it names the run';
COMMENT ON COLUMN run_spans.until_ms IS 'when the same instance''s next run started, Unix milliseconds, or 9223372036854775807 (the largest BIGINT) for its latest run; a row belongs to the run when run_ms <= ts_ms < until_ms';
