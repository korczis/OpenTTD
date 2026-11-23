-- Every evaluation of a panel service, one row per eval.<service>.<action> the service_eval.rhai service wrote to the panel's shared memory (rhai.shared events, research/mechanisms/service-self-evaluation.md): the verdict, its counts and posterior, the poor streak.
CREATE OR REPLACE VIEW service_eval AS
  SELECT instance, run_ms, seed, seq, game_date,
  json_extract_string(data, '$.value.service') AS service,
  json_extract_string(data, '$.value.action') AS action,
  json_extract_string(data, '$.value.endpoint') AS endpoint,
  json_extract_string(data, '$.value.verdict') AS verdict,
  TRY_CAST(json_extract_string(data, '$.value.n') AS BIGINT) AS n,
  TRY_CAST(json_extract_string(data, '$.value.successes') AS BIGINT) AS successes,
  TRY_CAST(json_extract_string(data, '$.value.runs') AS BIGINT) AS runs,
  TRY_CAST(json_extract_string(data, '$.value.rate') AS DOUBLE) AS rate,
  TRY_CAST(json_extract_string(data, '$.value.posterior.p_below_half') AS DOUBLE) AS p_below_half,
  TRY_CAST(json_extract_string(data, '$.value.median_relative_outcome') AS DOUBLE) AS median_relative_outcome,
  TRY_CAST(json_extract_string(data, '$.value.horizon') AS BIGINT) AS horizon,
  TRY_CAST(json_extract_string(data, '$.value.streak') AS BIGINT) AS streak,
  json_extract_string(data, '$.value.persistent') = 'true' AS persistent
  FROM run_events
  WHERE kind = 'rhai.shared' AND json_extract_string(data, '$.op') = 'set'
  AND regexp_full_match(coalesce(json_extract_string(data, '$.key'), ''), 'eval\.[a-z_]+\.[a-z_]+');

COMMENT ON VIEW service_eval IS 'Every evaluation of a panel service, one row per eval.<service>.<action> the service_eval.rhai service wrote to the panel''s shared memory (rhai.shared events, research/mechanisms/service-self-evaluation.md): the verdict, its counts and posterior, the poor streak.';
COMMENT ON COLUMN service_eval.instance IS 'the panel whose memory held the evaluation (run_events.instance)';
COMMENT ON COLUMN service_eval.run_ms IS 'the start of the run, Unix milliseconds (run_spans.run_ms)';
COMMENT ON COLUMN service_eval.seed IS 'the run''s map seed, or null (run_spans.seed)';
COMMENT ON COLUMN service_eval.seq IS 'the rhai.shared event''s sequence number, increasing within a run';
COMMENT ON COLUMN service_eval.game_date IS 'the game''s date YYYY-MM-DD of the evaluation';
COMMENT ON COLUMN service_eval.service IS 'the service judged: replan, watchdog, build_audit, rail_monitor or gap_watch (view service_actions)';
COMMENT ON COLUMN service_eval.action IS 'the action judged: proposal, hold, stall_alert, defect_flag, advice_flag or gap_alert';
COMMENT ON COLUMN service_eval.endpoint IS 'what an outcome is judged by: resolution (a detector) or relative_outcome (the share against its own trend)';
COMMENT ON COLUMN service_eval.verdict IS 'insufficient (fewer outcomes than the rule''s min_n), poor, unclear or helps (GET /api/services/scores rule)';
COMMENT ON COLUMN service_eval.n IS 'the outcomes judged';
COMMENT ON COLUMN service_eval.successes IS 'the outcomes that met the expected effect: resolved, or above the company''s own trend';
COMMENT ON COLUMN service_eval.runs IS 'the distinct runs the outcomes came from: how many independent games the count rests on';
COMMENT ON COLUMN service_eval.rate IS 'successes / n, null without outcomes';
COMMENT ON COLUMN service_eval.p_below_half IS 'the Jeffreys posterior probability that the success rate is under one half (approximate: horizons overlap)';
COMMENT ON COLUMN service_eval.median_relative_outcome IS 'the median of the scored outcomes'' share change minus the company''s pre-action trend, null without one';
COMMENT ON COLUMN service_eval.horizon IS 'the game months between an action''s state and its outcome';
COMMENT ON COLUMN service_eval.streak IS 'the evaluations in a row, each with newly matured outcomes, that judged it poor; 0 when not poor';
COMMENT ON COLUMN service_eval.persistent IS 'whether the streak reached service_eval.rhai PERSIST: then it is in eval.poor and the service''s log says POOR';
