-- Every company's last $months game months in the store's latest run, oldest first: the last sample
-- of each month's money, loan, net (money minus loan), vehicles and value (company_month). The
-- capital watch (tools/research/mcp-panel/rhai/capital_watch.rhai) takes the net at each quarter's
-- last month to split a quarter's operating cash flow into what was spent and what was kept.
-- Parameter: $months (how many months back from the run's newest one).
WITH latest AS (
  SELECT max(run_ms) AS run_ms FROM run_spans
),
m AS (
  SELECT *, CAST(substr(month, 1, 4) AS BIGINT) * 12 + CAST(substr(month, 6, 2) AS BIGINT) - 1 AS mi
  FROM company_month WHERE run_ms = (SELECT run_ms FROM latest)
)
SELECT company, name, month, money, loan, net, vehicles, value
FROM m
WHERE mi > (SELECT max(mi) FROM m) - $months
ORDER BY company, month;
