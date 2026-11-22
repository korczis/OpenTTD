-- The trailing window a kernel is fed from (src/kernel_feed.rs): the newest $days samples of every
-- company of the instance's latest run (a store keeps every run of its instance, and game ticks
-- restart). Parameters: $instance (e.g. panel-8800), $days.
SELECT company, game_tick, game_date, money, loan, value, net, vehicles
FROM samples
WHERE instance = $instance AND company IS NOT NULL AND game_tick IS NOT NULL
  AND ts_ms >= (SELECT coalesce(max(started_ms), 0) FROM runs WHERE instance = $instance)
QUALIFY row_number() OVER (PARTITION BY company ORDER BY game_tick DESC) <= $days
ORDER BY company, game_tick;
