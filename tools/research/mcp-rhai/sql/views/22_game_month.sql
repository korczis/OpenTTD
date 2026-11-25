-- Each instance's games across its runs: company_month's rows, chained over the runs that continue one game, each month from the newest run that has it.
-- A panel restarted on a save of its own game starts a new run with a new seed, so neither the
-- latest run nor the seed names the game. A run continues its predecessor (the same instance's run
-- before it that has months) when its first month comes after the predecessor's first and no later
-- than the month after the predecessor's last, and a company of the same id and name is in both at
-- the join (the predecessor's month least(first, last)). A game is a run that continues nothing
-- with every run that continues it in turn. Each run gives its months up to the first month of
-- the run that continues it, so a save loaded from an earlier date replaces the months after it.
CREATE OR REPLACE VIEW game_month AS
  WITH spans AS (
    SELECT instance, run_ms, min(month) AS lo, max(month) AS hi FROM company_month GROUP BY ALL
  ),
  linked AS (
    SELECT *, lag(run_ms) OVER w AS prev_run, lag(lo) OVER w AS prev_lo, lag(hi) OVER w AS prev_hi
    FROM spans WINDOW w AS (PARTITION BY instance ORDER BY run_ms)
  ),
  continued AS (
    SELECT l.instance, l.run_ms, l.lo,
      (l.prev_run IS NOT NULL AND l.lo > l.prev_lo
       AND CAST(substr(l.lo, 1, 4) AS BIGINT) * 12 + CAST(substr(l.lo, 6, 2) AS BIGINT)
           <= CAST(substr(l.prev_hi, 1, 4) AS BIGINT) * 12 + CAST(substr(l.prev_hi, 6, 2) AS BIGINT) + 1
       AND EXISTS (SELECT 1 FROM company_month a JOIN company_month b ON a.company = b.company AND a.name = b.name
                   WHERE a.instance = l.instance AND a.run_ms = l.run_ms AND a.month = l.lo
                     AND b.instance = l.instance AND b.run_ms = l.prev_run AND b.month = least(l.lo, l.prev_hi)))
      AS continues
    FROM linked l
  ),
  games AS (
    SELECT instance, run_ms, lo,
      max(CASE WHEN continues THEN NULL ELSE run_ms END) OVER (PARTITION BY instance ORDER BY run_ms) AS game_ms,
      lead(lo) OVER (PARTITION BY instance ORDER BY run_ms) AS next_lo,
      lead(continues) OVER (PARTITION BY instance ORDER BY run_ms) AS next_continues
    FROM continued
  )
  SELECT m.instance, g.game_ms, m.run_ms, m.seed, m.company, m.name, m.month, m.game_date, m.value, m.money,
    m.loan, m.net, m.vehicles, m.value_change, m.money_change, m.vehicles_change
  FROM company_month m JOIN games g ON m.instance = g.instance AND m.run_ms = g.run_ms
  WHERE NOT coalesce(g.next_continues, false) OR m.month < g.next_lo;

COMMENT ON VIEW game_month IS 'Each instance''s games across its runs: company_month''s rows, chained over the runs that continue one game (a later first month, at most one month after the run before ends, a company of the same id and name at the join), each month from the newest run that has it.';
COMMENT ON COLUMN game_month.instance IS 'the instance sampled, the publisher of the runs (company_month.instance)';
COMMENT ON COLUMN game_month.game_ms IS 'the start of the game''s first run, Unix milliseconds: with instance it names the game across the runs that continue it';
COMMENT ON COLUMN game_month.run_ms IS 'the start of the run this month comes from, Unix milliseconds (company_month.run_ms)';
COMMENT ON COLUMN game_month.seed IS 'the seed that run recorded, or null (company_month.seed); a run restarted on a save records another';
COMMENT ON COLUMN game_month.company IS 'the company id, 0-based (company 1 in the game is 0)';
COMMENT ON COLUMN game_month.name IS 'the company''s name at its last sample of the month';
COMMENT ON COLUMN game_month.month IS 'the game month YYYY-MM';
COMMENT ON COLUMN game_month.game_date IS 'the latest game date YYYY-MM-DD sampled for the company in the month';
COMMENT ON COLUMN game_month.value IS 'the company value as the game computes it, at its last sample of the month';
COMMENT ON COLUMN game_month.money IS 'cash, in the game''s internal currency, at its last sample of the month';
COMMENT ON COLUMN game_month.loan IS 'the loan at its last sample of the month';
COMMENT ON COLUMN game_month.net IS 'money minus loan at its last sample of the month';
COMMENT ON COLUMN game_month.vehicles IS 'the number of vehicles at its last sample of the month';
COMMENT ON COLUMN game_month.value_change IS 'value minus the value of the company''s previous month in the same run (company_month.value_change), null in a run''s first month';
COMMENT ON COLUMN game_month.money_change IS 'money minus the money of the company''s previous month in the same run, null in a run''s first month';
COMMENT ON COLUMN game_month.vehicles_change IS 'vehicles minus the vehicles of the company''s previous month in the same run, null in a run''s first month';
