-- How many events of each kind each game month of each run raised.
CREATE OR REPLACE VIEW kind_month AS
  SELECT instance, run_ms, substr(game_date, 1, 7) AS month, kind, count(*) AS n
  FROM run_events WHERE game_date IS NOT NULL GROUP BY ALL;

COMMENT ON VIEW kind_month IS 'How many events of each kind each game month of each run raised.';
