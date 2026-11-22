-- The calibration readings of a store, newest first (src/calibrate.rs, I0082): every
-- plan.calibration event with the seed of the run it belongs to (the newest run of its instance
-- started before it) and the map size its game had. Parameters: $seed and $map narrow it to one
-- game (both null: every game), $limit (how many readings).
WITH r AS (
  SELECT instance, seed, started_ms,
         lead(started_ms) OVER (PARTITION BY instance ORDER BY started_ms) AS until_ms
  FROM runs
)
SELECT e.instance, e.seq, e.ts_ms, r.seed,
       json_extract_string(e.data, '$.inputs.live.game.map') AS map,
       CAST(json_extract(e.data, '$.choice.reading.lines') AS VARCHAR) AS lines
FROM events e
LEFT JOIN r ON r.instance = e.instance AND e.ts_ms >= r.started_ms
           AND (r.until_ms IS NULL OR e.ts_ms < r.until_ms)
WHERE e.kind = 'plan.calibration'
  AND ($seed IS NULL OR r.seed = $seed)
  AND ($map IS NULL OR json_extract_string(e.data, '$.inputs.live.game.map') = $map)
ORDER BY e.ts_ms DESC
LIMIT $limit;
