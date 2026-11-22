-- Every event with the run it belongs to (run_ms, seed) beside its own columns.
CREATE OR REPLACE VIEW run_events AS
  SELECT r.run_ms, r.seed, e.* FROM events e
  JOIN run_spans r ON e.instance = r.instance AND e.ts_ms >= r.run_ms AND e.ts_ms < r.until_ms;

COMMENT ON VIEW run_events IS 'Every event with the run it belongs to (run_ms, seed) beside its own columns.';
COMMENT ON COLUMN run_events.run_ms IS 'the start of the run the event falls in, Unix milliseconds (run_spans.run_ms)';
COMMENT ON COLUMN run_events.seed IS 'the run''s map seed, or null (run_spans.seed)';
COMMENT ON COLUMN run_events.instance IS 'the instance the event is about (events.instance), the publisher of the run';
COMMENT ON COLUMN run_events.seq IS 'the publisher''s sequence number, increasing within a run';
COMMENT ON COLUMN run_events.ts_ms IS 'when it was published, Unix milliseconds';
COMMENT ON COLUMN run_events.kind IS 'the event kind, e.g. game.month, news.shown, plan.portfolio';
COMMENT ON COLUMN run_events.game_tick IS 'the game''s tick when it was published, or null';
COMMENT ON COLUMN run_events.game_date IS 'the game''s date YYYY-MM-DD, or null';
COMMENT ON COLUMN run_events.data IS 'the payload as JSON; read fields with data->>''field''';
