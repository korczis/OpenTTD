-- Baselines the services derive their thresholds from (research/mechanisms/service-self-evaluation.md),
-- read from comparable runs BEFORE the store's latest one only, so a running game never calibrates on
-- itself. Comparable: not a loaded continuation (a run whose first sample lies within a month of the
-- previous run's last one and after that run's first: a saved game played on, whatever seed the
-- loader gave it, e.g. 31337 continuing 15805), and with a lineup of about the latest run's size (as
-- many companies as it has sampled, give or take two; any size while it has no sample yet). The
-- game speed is not in the store and is not matched. One row per metric: n, the runs it came from,
-- and its 50th, 75th and 90th percentile (quantile_cont).
--   quiet_days        completed quiet spells: game days between two changes of a company's vehicle count
--                     (samples, 74 ticks a day); the first and the last, still open, spell of each
--                     company are censored and left out. A proxy of the watchdog's progress signature,
--                     which also counts vehicle types and stations.
--   share_drop        how far a company's share of every company's value fell from one game month to the
--                     next, over the months it fell (company_month).
--   leader_ratio_drop how far a company's value as a share of the leader's (the most valuable company that
--                     month) fell over $months game months, over the windows it fell; gap_watch.rhai's
--                     share_drop measure (WINDOW months), the leader itself left out.
-- Parameter: $months (months).
WITH latest AS (
  SELECT max(run_ms) AS run_ms FROM run_spans
),
spans AS (
  SELECT r.instance, r.run_ms,
         min(CAST(substr(x.game_date, 1, 4) AS BIGINT) * 12 + CAST(substr(x.game_date, 6, 2) AS BIGINT)) AS first_mi,
         max(CAST(substr(x.game_date, 1, 4) AS BIGINT) * 12 + CAST(substr(x.game_date, 6, 2) AS BIGINT)) AS last_mi,
         count(DISTINCT x.company) AS companies
  FROM run_spans r JOIN samples x ON x.instance = r.instance AND x.ts_ms >= r.run_ms AND x.ts_ms < r.until_ms
  GROUP BY ALL
),
lineup AS (
  SELECT max(companies) AS companies FROM spans WHERE run_ms = (SELECT run_ms FROM latest)
),
eligible AS (
  SELECT instance, run_ms
  FROM (SELECT *, lag(last_mi) OVER (PARTITION BY instance ORDER BY run_ms) AS prev_last_mi,
               lag(first_mi) OVER (PARTITION BY instance ORDER BY run_ms) AS prev_first_mi FROM spans)
  WHERE run_ms < (SELECT run_ms FROM latest)
    AND NOT coalesce(first_mi > prev_first_mi AND first_mi >= prev_last_mi - 1 AND first_mi <= prev_last_mi + 1, false)
    AND ((SELECT companies FROM lineup) IS NULL OR abs(companies - (SELECT companies FROM lineup)) <= 2)
),
s AS (
  SELECT r.instance, r.run_ms, x.company, x.game_tick, x.vehicles,
         lag(x.vehicles) OVER (PARTITION BY r.instance, r.run_ms, x.company ORDER BY x.game_tick, x.ts_ms) AS prev
  FROM samples x JOIN run_spans r ON x.instance = r.instance AND x.ts_ms >= r.run_ms AND x.ts_ms < r.until_ms
  WHERE (r.instance, r.run_ms) IN (SELECT (instance, run_ms) FROM eligible)
),
changes AS (
  SELECT instance, run_ms, company, game_tick,
         lag(game_tick) OVER (PARTITION BY instance, run_ms, company ORDER BY game_tick) AS prev_tick
  FROM s WHERE prev IS NOT NULL AND vehicles <> prev
),
quiet AS (
  SELECT 'quiet_days' AS metric, run_ms, (game_tick - prev_tick) / 74.0 AS x
  FROM changes WHERE prev_tick IS NOT NULL AND game_tick > prev_tick
),
f AS (
  SELECT instance, run_ms, company,
         CAST(substr(month, 1, 4) AS BIGINT) * 12 + CAST(substr(month, 6, 2) AS BIGINT) AS mi,
         value / nullif(sum(value) OVER (PARTITION BY instance, run_ms, month), 0) AS share,
         CASE WHEN value < max(value) OVER (PARTITION BY instance, run_ms, month)
              THEN value / nullif(max(value) OVER (PARTITION BY instance, run_ms, month), 0) END AS ratio
  FROM company_month WHERE (instance, run_ms) IN (SELECT (instance, run_ms) FROM eligible)
),
ratio_drops AS (
  SELECT 'leader_ratio_drop' AS metric, a.run_ms, a.ratio - b.ratio AS x
  FROM f a JOIN f b ON b.instance = a.instance AND b.run_ms = a.run_ms AND b.company = a.company
                   AND b.mi = a.mi + $months
  WHERE a.ratio IS NOT NULL AND b.ratio IS NOT NULL AND b.ratio < a.ratio
),
drops AS (
  SELECT 'share_drop' AS metric, run_ms, -(share - prev_share) AS x
  FROM (SELECT *, lag(share) OVER (PARTITION BY instance, run_ms, company ORDER BY mi) AS prev_share,
               lag(mi) OVER (PARTITION BY instance, run_ms, company ORDER BY mi) AS prev_mi FROM f)
  WHERE prev_mi = mi - 1 AND share < prev_share
),
x AS (
  SELECT * FROM quiet UNION ALL SELECT * FROM drops UNION ALL SELECT * FROM ratio_drops
)
SELECT metric, count(*) AS n, count(DISTINCT run_ms) AS runs,
       quantile_cont(x, 0.5) AS p50, quantile_cont(x, 0.75) AS p75, quantile_cont(x, 0.9) AS p90
FROM x GROUP BY metric ORDER BY metric;
