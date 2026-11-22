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
COMMENT ON COLUMN company_month.instance IS 'the instance sampled (samples.instance), the publisher of the run';
COMMENT ON COLUMN company_month.run_ms IS 'the start of the run the samples fall in, Unix milliseconds (run_spans.run_ms)';
COMMENT ON COLUMN company_month.seed IS 'the run''s map seed, or null (run_spans.seed)';
COMMENT ON COLUMN company_month.company IS 'the company id, 0-based (company 1 in the game is 0)';
COMMENT ON COLUMN company_month.name IS 'the company''s name at its last sample of the month';
COMMENT ON COLUMN company_month.month IS 'the game month YYYY-MM (the first 7 characters of samples.game_date)';
COMMENT ON COLUMN company_month.game_date IS 'the latest game date YYYY-MM-DD sampled for the company in the month';
COMMENT ON COLUMN company_month.value IS 'the company value as the game computes it, at its last sample of the month';
COMMENT ON COLUMN company_month.money IS 'cash, in the game''s internal currency, at its last sample of the month';
COMMENT ON COLUMN company_month.loan IS 'the loan at its last sample of the month';
COMMENT ON COLUMN company_month.net IS 'money minus loan at its last sample of the month';
COMMENT ON COLUMN company_month.vehicles IS 'the number of vehicles at its last sample of the month';
COMMENT ON COLUMN company_month.value_change IS 'value minus the value of the company''s previous sampled month in the same run, null in its first month';
COMMENT ON COLUMN company_month.money_change IS 'money minus the money of the company''s previous sampled month in the same run, null in its first month';
COMMENT ON COLUMN company_month.vehicles_change IS 'vehicles minus the vehicles of the company''s previous sampled month in the same run, null in its first month';
