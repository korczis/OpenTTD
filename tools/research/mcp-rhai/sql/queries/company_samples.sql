-- A company's recorded samples in every run of the store (prismatic-history/v1, I0085): the first
-- sample of each game month per run, each with its run (run_ms, seed) and the aggressiveness the
-- latest plan.directive of that run had set for the company by then, with that directive's outcome
-- (applied, pending, unverified or sent); null before any, the AI's own default. Parameter:
-- $company (0-based id).
WITH r AS (
  SELECT instance, seed, started_ms,
         lead(started_ms) OVER (PARTITION BY instance ORDER BY started_ms) AS until_ms
  FROM runs
),
s AS (
  SELECT x.instance, r.seed, r.started_ms AS run_ms, x.ts_ms, x.game_tick, x.game_date, x.value,
         x.money, x.loan, x.net, x.vehicles
  FROM samples x
  JOIN r ON r.instance = x.instance AND x.ts_ms >= r.started_ms
        AND (r.until_ms IS NULL OR x.ts_ms < r.until_ms)
  WHERE x.company = $company
  QUALIFY row_number() OVER (PARTITION BY x.instance, r.started_ms, substr(x.game_date, 1, 7)
                             ORDER BY x.game_tick, x.ts_ms) = 1
),
a AS (
  SELECT e.instance, r.started_ms AS run_ms, e.ts_ms,
         TRY_CAST(json_extract_string(e.data, '$.choice.settings.aggressiveness') AS BIGINT)
           AS aggressiveness,
         json_extract_string(e.data, '$.choice.outcome') AS directive_outcome
  FROM events e
  JOIN r ON r.instance = e.instance AND e.ts_ms >= r.started_ms
        AND (r.until_ms IS NULL OR e.ts_ms < r.until_ms)
  WHERE e.kind = 'plan.directive'
    AND json_extract_string(e.data, '$.choice.outcome') IN ('applied', 'pending', 'unverified', 'sent')
    AND TRY_CAST(json_extract_string(e.data, '$.choice.company_id') AS BIGINT) = $company
    AND json_extract_string(e.data, '$.choice.settings.aggressiveness') IS NOT NULL
)
SELECT s.instance, s.seed, s.run_ms, s.game_tick, s.game_date, s.value, s.money, s.loan, s.net,
       s.vehicles, a.aggressiveness, a.directive_outcome
FROM s ASOF LEFT JOIN a ON s.instance = a.instance AND s.run_ms = a.run_ms AND s.ts_ms >= a.ts_ms
ORDER BY s.instance, s.run_ms, s.game_tick;
