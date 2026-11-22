-- Every planning decision (plan.* events): its kind, decision, choice and version.
CREATE OR REPLACE VIEW plan_decisions AS
  SELECT instance, run_ms, seed, seq, game_date, kind, data->>'decision' AS decision,
  data->'choice' AS choice, data->'version' AS version
  FROM run_events WHERE kind LIKE 'plan.%';

COMMENT ON VIEW plan_decisions IS 'Every planning decision (plan.* events): its kind, decision, choice and version.';
