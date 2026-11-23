-- Every action a panel service took, derived from the shared-memory writes it already makes (rhai.shared events, research/mechanisms/service-self-evaluation.md): a replan proposal or hold, a watchdog stall alert, a build_audit defect flag, a rail_monitor advice flag, a gap_watch alert; each with the company's state the month before and the effect the action is expected to have.
CREATE OR REPLACE VIEW service_actions AS
  WITH w AS (
    SELECT instance, run_ms, seed, seq, game_date,
    json_extract_string(data, '$.key') AS key, json_extract(data, '$.value') AS v
    FROM run_events
    WHERE kind = 'rhai.shared' AND json_extract_string(data, '$.op') = 'set'
    AND regexp_full_match(coalesce(json_extract_string(data, '$.key'), ''), '(replan|watchdog|audit|advice|gap)\.[0-9]+')
  ), t AS (
    SELECT *, split_part(key, '.', 1) AS family, CAST(split_part(key, '.', 2) AS BIGINT) AS company,
    lag(v) OVER (PARTITION BY instance, run_ms, key ORDER BY seq) AS prev
    FROM w
  ), a AS (
    SELECT *,
    CASE family
      WHEN 'replan' THEN CASE WHEN coalesce(json_array_length(json_extract(v, '$.pending')), 0) > 0 THEN 'proposal' ELSE 'hold' END
      WHEN 'watchdog' THEN CASE WHEN json_extract_string(v, '$.stalled') = 'true'
        AND coalesce(json_extract_string(prev, '$.stalled'), 'false') <> 'true' THEN 'stall_alert' END
      WHEN 'audit' THEN CASE WHEN coalesce(json_array_length(json_extract(v, '$.new_keys')), 0) > 0 THEN 'defect_flag' END
      WHEN 'advice' THEN CASE WHEN TRY_CAST(json_extract_string(v, '$.total') AS BIGINT)
        > coalesce(TRY_CAST(json_extract_string(prev, '$.total') AS BIGINT), 0) THEN 'advice_flag' END
      WHEN 'gap' THEN CASE WHEN TRY_CAST(json_extract_string(v, '$.level') AS BIGINT)
        > coalesce(TRY_CAST(json_extract_string(prev, '$.level') AS BIGINT), 0) THEN 'gap_alert' END
    END AS action
    FROM t
  ), f AS (
    SELECT instance, run_ms, company, CAST(substr(month, 1, 4) AS BIGINT) * 12 + CAST(substr(month, 6, 2) AS BIGINT) AS mi,
    value, vehicles,
    value / nullif(sum(value) OVER (PARTITION BY instance, run_ms, month), 0) AS share,
    max(value) OVER (PARTITION BY instance, run_ms, month) - value AS gap
    FROM company_month
  )
  SELECT a.instance || ':' || a.run_ms || ':' || a.seq AS id, a.instance, a.run_ms, a.seed, a.seq, a.game_date,
  substr(a.game_date, 1, 7) AS month, a.company,
  CASE a.family WHEN 'audit' THEN 'build_audit' WHEN 'advice' THEN 'rail_monitor' WHEN 'gap' THEN 'gap_watch' ELSE a.family END AS service,
  a.action, a.key,
  f.value, f.share, f.gap, f.vehicles,
  CASE a.family
    WHEN 'replan' THEN json_object('why', json_extract_string(a.v, '$.model.why'), 'rank', json_extract(a.v, '$.rank'),
      'of', json_extract(a.v, '$.of'), 'pending', json_extract(a.v, '$.pending'), 'held', json_extract(a.v, '$.held'))
    WHEN 'watchdog' THEN json_object('quiet_days', json_extract(a.v, '$.quiet_days'),
      'threshold_days', json_extract(a.v, '$.threshold_days'), 'reasons', json_extract(a.v, '$.reasons'))
    WHEN 'audit' THEN json_object('new_keys', json_extract(a.v, '$.new_keys'), 'defects_total', json_extract(a.v, '$.defects_total'))
    WHEN 'advice' THEN json_object('total', json_extract(a.v, '$.total'), 'by_advice', json_extract(a.v, '$.by_advice'))
    WHEN 'gap' THEN json_object('level', json_extract(a.v, '$.level'), 'flags', json_extract(a.v, '$.flags'))
  END AS detail,
  CASE a.action
    WHEN 'proposal' THEN 'the proposed settings, once approved, lift the company''s share of the field''s value above its own trend'
    WHEN 'hold' THEN 'holding the current plan keeps the company''s share of the field''s value at or above its own trend'
    WHEN 'stall_alert' THEN 'the stalled company builds again and its share recovers above its own trend'
    WHEN 'defect_flag' THEN 'the flagged defects are fixed and the share does not fall below its own trend'
    WHEN 'advice_flag' THEN 'the advised vehicles are dealt with and the share does not fall below its own trend'
    WHEN 'gap_alert' THEN 'the gap to the leader narrows: the share rises above its own trend'
  END AS expected
  FROM a LEFT JOIN f ON f.instance = a.instance AND f.run_ms = a.run_ms AND f.company = a.company
  AND f.mi = CAST(substr(a.game_date, 1, 4) AS BIGINT) * 12 + CAST(substr(a.game_date, 6, 2) AS BIGINT) - 1
  WHERE a.action IS NOT NULL;

COMMENT ON VIEW service_actions IS 'Every action a panel service took, derived from the shared-memory writes it already makes (rhai.shared events, research/mechanisms/service-self-evaluation.md): a replan proposal or hold, a watchdog stall alert, a build_audit defect flag, a rail_monitor advice flag, a gap_watch alert; each with the company''s state the month before and the effect the action is expected to have.';
COMMENT ON COLUMN service_actions.id IS 'the action''s id, <instance>:<run_ms>:<seq> of the rhai.shared event that recorded it';
COMMENT ON COLUMN service_actions.instance IS 'the panel whose memory held the write (run_events.instance)';
COMMENT ON COLUMN service_actions.run_ms IS 'the start of the run, Unix milliseconds (run_spans.run_ms)';
COMMENT ON COLUMN service_actions.seed IS 'the run''s map seed, or null (run_spans.seed)';
COMMENT ON COLUMN service_actions.seq IS 'the rhai.shared event''s sequence number, increasing within a run';
COMMENT ON COLUMN service_actions.game_date IS 'the game''s date YYYY-MM-DD when the service wrote it';
COMMENT ON COLUMN service_actions.month IS 'the game month YYYY-MM of the action';
COMMENT ON COLUMN service_actions.company IS 'the company id, 0-based, from the key <family>.<company>';
COMMENT ON COLUMN service_actions.service IS 'the service: replan, watchdog, build_audit (audit.<c>), rail_monitor (advice.<c>) or gap_watch (gap.<c>)';
COMMENT ON COLUMN service_actions.action IS 'proposal (replan filed approvals: pending non-empty), hold (replan filed none), stall_alert (watchdog stalled turned true), defect_flag (build_audit new_keys non-empty), advice_flag (the advice tally total rose), gap_alert (gap_watch level rose)';
COMMENT ON COLUMN service_actions.key IS 'the shared key written, e.g. replan.0';
COMMENT ON COLUMN service_actions.value IS 'the company''s value at its last sample of the month before the action (company_month.value): the state the service acted on; null without samples';
COMMENT ON COLUMN service_actions.share IS 'the company''s value over the sum of every company''s value the month before the action, null when the sum is 0 or there are no samples';
COMMENT ON COLUMN service_actions.gap IS 'the largest company value the month before the action minus the company''s own, 0 for the leader';
COMMENT ON COLUMN service_actions.vehicles IS 'the company''s vehicles at its last sample of the month before the action';
COMMENT ON COLUMN service_actions.detail IS 'what the service observed, as JSON: replan why, rank, of, pending, held; watchdog quiet_days, threshold_days, reasons; build_audit new_keys, defects_total; rail_monitor total, by_advice; gap_watch level, flags';
COMMENT ON COLUMN service_actions.expected IS 'the effect the action is expected to have, scored against the share of the field''s value the horizon later (query service_outcomes)';
