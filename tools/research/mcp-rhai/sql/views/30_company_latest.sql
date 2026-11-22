-- Each company's latest month of each run (company_month's columns) and its rank by value among that run's companies.
CREATE OR REPLACE VIEW company_latest AS
  SELECT *, rank() OVER (PARTITION BY instance, run_ms ORDER BY value DESC) AS rank
  FROM (SELECT * FROM company_month
  QUALIFY row_number() OVER (PARTITION BY instance, run_ms, company ORDER BY month DESC) = 1);

COMMENT ON VIEW company_latest IS 'Each company''s latest month of each run (company_month''s columns) and its rank by value among that run''s companies.';
COMMENT ON COLUMN company_latest.instance IS 'the instance sampled, the publisher of the run (company_month.instance)';
COMMENT ON COLUMN company_latest.run_ms IS 'the start of the run, Unix milliseconds (run_spans.run_ms)';
COMMENT ON COLUMN company_latest.seed IS 'the run''s map seed, or null (run_spans.seed)';
COMMENT ON COLUMN company_latest.company IS 'the company id, 0-based (company 1 in the game is 0)';
COMMENT ON COLUMN company_latest.name IS 'the company''s name at its last sample of its latest month';
COMMENT ON COLUMN company_latest.month IS 'the company''s latest game month YYYY-MM in the run';
COMMENT ON COLUMN company_latest.game_date IS 'the latest game date YYYY-MM-DD sampled for the company in that month';
COMMENT ON COLUMN company_latest.value IS 'the company value as the game computes it, at its last sample of its latest month';
COMMENT ON COLUMN company_latest.money IS 'cash, in the game''s internal currency, at its last sample of its latest month';
COMMENT ON COLUMN company_latest.loan IS 'the loan at its last sample of its latest month';
COMMENT ON COLUMN company_latest.net IS 'money minus loan at its last sample of its latest month';
COMMENT ON COLUMN company_latest.vehicles IS 'the number of vehicles at its last sample of its latest month';
COMMENT ON COLUMN company_latest.value_change IS 'value minus the value of the company''s previous sampled month in the run, null when it has only one';
COMMENT ON COLUMN company_latest.money_change IS 'money minus the money of the company''s previous sampled month in the run, null when it has only one';
COMMENT ON COLUMN company_latest.vehicles_change IS 'vehicles minus the vehicles of the company''s previous sampled month in the run, null when it has only one';
COMMENT ON COLUMN company_latest.rank IS 'the company''s rank by value among the run''s companies, each at its latest month: 1 is the highest, ties share a rank';
