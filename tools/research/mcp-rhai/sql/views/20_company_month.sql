-- Each company's last sample of each game month (YYYY-MM) of each run: value, money, loan, net, vehicles, with the change from the month before.
CREATE OR REPLACE VIEW company_month AS
  SELECT *, value - lag(value) OVER w AS value_change, money - lag(money) OVER w AS money_change,
  vehicles - lag(vehicles) OVER w AS vehicles_change
  FROM (SELECT r.instance, r.run_ms, r.seed, s.company, last(s.name ORDER BY s.ts_ms) AS name,
  substr(s.game_date, 1, 7) AS month, max(s.game_date) AS game_date,
  last(s.value ORDER BY s.ts_ms) AS value, last(s.money ORDER BY s.ts_ms) AS money,
  last(s.loan ORDER BY s.ts_ms) AS loan, last(s.net ORDER BY s.ts_ms) AS net,
  last(s.vehicles ORDER BY s.ts_ms) AS vehicles
  FROM samples s JOIN run_spans r ON s.instance = r.instance AND s.ts_ms >= r.run_ms AND s.ts_ms < r.until_ms
  GROUP BY ALL)
  WINDOW w AS (PARTITION BY instance, run_ms, company ORDER BY month);

COMMENT ON VIEW company_month IS 'Each company''s last sample of each game month (YYYY-MM) of each run: value, money, loan, net, vehicles, with the change from the month before.';
