-- The duel outcomes of a store (src/duel_outcomes.rs, I0087): the plan.duel verdicts of instances
-- with a verdict of at least $min_months months, and the plan.bot_settings events of the same
-- instances, in order, so each verdict joins the settings its run started the planned bot with.
-- Parameter: $min_months.
WITH d AS (
  SELECT DISTINCT instance FROM events
  WHERE kind = 'plan.duel'
    AND TRY_CAST(json_extract_string(data, '$.choice.months') AS INTEGER) >= $min_months
)
SELECT instance, seq, ts_ms, kind, CAST(data AS VARCHAR) AS data
FROM events
WHERE instance IN (SELECT instance FROM d) AND kind IN ('plan.duel', 'plan.bot_settings')
ORDER BY instance, ts_ms, seq;
