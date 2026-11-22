-- Every bus event in bus order (envelope prismatic-bus/v1); the kinds are the KINDS table of panel_events.rs, listed by GET /api/bus.
CREATE TABLE IF NOT EXISTS events (
  instance VARCHAR,
  seq BIGINT,
  ts_ms BIGINT,
  kind VARCHAR,
  game_tick BIGINT,
  game_date VARCHAR,
  data JSON
);

COMMENT ON TABLE events IS 'Every bus event in bus order (envelope prismatic-bus/v1); the kinds are the KINDS table of panel_events.rs, listed by GET /api/bus.';
COMMENT ON COLUMN events.instance IS 'the instance the event is about';
COMMENT ON COLUMN events.seq IS 'the publisher''s sequence number, increasing within a run';
COMMENT ON COLUMN events.ts_ms IS 'when it was published, Unix milliseconds';
COMMENT ON COLUMN events.kind IS 'the event kind, e.g. game.month, news.shown, plan.portfolio';
COMMENT ON COLUMN events.game_tick IS 'the game''s tick when it was published, or null';
COMMENT ON COLUMN events.game_date IS 'the game''s date YYYY-MM-DD, or null';
COMMENT ON COLUMN events.data IS 'the payload as JSON; read fields with data->>''field''';
