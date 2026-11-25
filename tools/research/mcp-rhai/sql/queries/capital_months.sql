-- Every company's last $months game months in the store's current game, oldest first: the last
-- sample of each month's money, loan, net (money minus loan), vehicles and value (game_month: the
-- runs that continue one game, so a panel restarted on its own save keeps the months before it).
-- The capital watch (tools/research/mcp-panel/rhai/capital_watch.rhai) takes the net at each
-- quarter's last month to split a quarter's operating cash flow into what was spent and what was
-- kept. The current game is the one of the store's latest run; none while that run has no month.
-- Parameter: $months (how many months back from the game's newest one).
WITH latest AS (
  SELECT instance, run_ms FROM run_spans ORDER BY run_ms DESC LIMIT 1
),
game AS (
  SELECT g.instance, g.game_ms FROM game_month g JOIN latest l ON g.instance = l.instance AND g.run_ms = l.run_ms
  LIMIT 1
),
m AS (
  SELECT g.*, CAST(substr(g.month, 1, 4) AS BIGINT) * 12 + CAST(substr(g.month, 6, 2) AS BIGINT) - 1 AS mi
  FROM game_month g JOIN game USING (instance, game_ms)
)
SELECT company, name, month, money, loan, net, vehicles, value
FROM m
WHERE mi > (SELECT max(mi) FROM m) - $months
ORDER BY company, month;
