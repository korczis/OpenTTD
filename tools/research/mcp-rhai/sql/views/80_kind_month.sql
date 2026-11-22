-- How many events of each kind each game month of each run raised.
CREATE OR REPLACE VIEW kind_month AS
  SELECT instance, run_ms, substr(game_date, 1, 7) AS month, kind, count(*) AS n
  FROM run_events WHERE game_date IS NOT NULL GROUP BY ALL;

COMMENT ON VIEW kind_month IS 'How many events of each kind each game month of each run raised.';
COMMENT ON COLUMN kind_month.instance IS 'the instance the events are about (run_events.instance)';
COMMENT ON COLUMN kind_month.run_ms IS 'the start of the run, Unix milliseconds (run_spans.run_ms)';
COMMENT ON COLUMN kind_month.month IS 'the game month YYYY-MM (the first 7 characters of game_date); events without a game date are left out';
COMMENT ON COLUMN kind_month.kind IS 'the event kind, e.g. game.month, news.shown, plan.portfolio';
COMMENT ON COLUMN kind_month.n IS 'how many events of that kind the run raised in that month';
