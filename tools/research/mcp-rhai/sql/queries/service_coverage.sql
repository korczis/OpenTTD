-- Did the services react while a company was losing ground? Deterioration episodes derived from the
-- samples alone, never from a service's own writes: consecutive game months in which a company's share
-- of every company's value fell by at least $drop from the month before, merged into one episode. Per
-- service and episode: whether the service acted on that company (view service_actions, replan holds
-- excluded: a hold is no reaction) from $lead months before the episode's first month to its last month,
-- and the months from the episode's start to the first such action. Per service the actions outside any
-- episode are counted too (alert burden). One row per service and run. Parameters: $drop (a share, e.g.
-- 0.02), $lead (months).
WITH f AS (
  SELECT instance, run_ms, company,
         CAST(substr(month, 1, 4) AS BIGINT) * 12 + CAST(substr(month, 6, 2) AS BIGINT) AS mi,
         value / nullif(sum(value) OVER (PARTITION BY instance, run_ms, month), 0) AS share
  FROM company_month
),
dropping AS (
  SELECT instance, run_ms, company, mi
  FROM (SELECT *, share - lag(share) OVER (PARTITION BY instance, run_ms, company ORDER BY mi) AS change,
               lag(mi) OVER (PARTITION BY instance, run_ms, company ORDER BY mi) AS prev_mi FROM f)
  WHERE prev_mi = mi - 1 AND change <= -$drop
),
ep AS (
  SELECT instance, run_ms, company, min(mi) AS first_mi, max(mi) AS last_mi
  FROM (SELECT *, mi - row_number() OVER (PARTITION BY instance, run_ms, company ORDER BY mi) AS grp FROM dropping)
  GROUP BY instance, run_ms, company, grp
),
act AS (
  SELECT instance, run_ms, service, company,
         CAST(substr(month, 1, 4) AS BIGINT) * 12 + CAST(substr(month, 6, 2) AS BIGINT) AS mi
  FROM service_actions WHERE action <> 'hold'
),
services AS (
  SELECT DISTINCT instance, run_ms, service FROM act
),
hit AS (
  SELECT s.service, ep.instance, ep.run_ms, ep.company, ep.first_mi, min(act.mi) AS first_action_mi
  FROM ep JOIN services s ON s.instance = ep.instance AND s.run_ms = ep.run_ms
  LEFT JOIN act ON act.instance = ep.instance AND act.run_ms = ep.run_ms AND act.service = s.service
               AND act.company = ep.company AND act.mi BETWEEN ep.first_mi - $lead AND ep.last_mi
  GROUP BY ALL
),
outside AS (
  SELECT act.instance, act.run_ms, act.service, count(*) AS actions,
         count_if(NOT EXISTS (SELECT 1 FROM ep WHERE ep.instance = act.instance AND ep.run_ms = act.run_ms
                                AND ep.company = act.company
                                AND act.mi BETWEEN ep.first_mi - $lead AND ep.last_mi)) AS outside_episodes
  FROM act GROUP BY ALL
)
SELECT o.service, o.instance, o.run_ms, count(hit.first_mi) AS episodes,
       count(hit.first_action_mi) AS covered,
       count(hit.first_action_mi) / nullif(count(hit.first_mi), 0) AS coverage,
       median(hit.first_action_mi - hit.first_mi) AS median_latency_months,
       o.actions, o.outside_episodes
FROM outside o
LEFT JOIN hit ON hit.instance = o.instance AND hit.run_ms = o.run_ms AND hit.service = o.service
GROUP BY o.service, o.instance, o.run_ms, o.actions, o.outside_episodes
ORDER BY o.run_ms, o.service;
