-- Every service action (view service_actions, one per service, action, company and month) joined to
-- what followed it $horizon game months later in the same run (research/mechanisms/service-self-evaluation.md):
-- share_after, delta_share, the company's pre-action baseline (the median $horizon-month share change of
-- its windows that ended by the action's state month; no future data), relative_outcome (delta_share minus
-- the baseline: positive is above its own trend), value_change, and for a detector its resolution (a
-- watchdog stall followed by progress, build_audit defects that fell, a gap_watch level that fell, within
-- the horizon) with the latency in game days. status: scored, pending (the horizon is not reached in the
-- latest run), ended (the run stopped first), company_gone (the run went on without the company),
-- no_state (no sample the month before), no_baseline (no earlier window). Parameter: $horizon (months).
WITH f AS (
  SELECT instance, run_ms, company, month,
         CAST(substr(month, 1, 4) AS BIGINT) * 12 + CAST(substr(month, 6, 2) AS BIGINT) AS mi,
         value, vehicles,
         value / nullif(sum(value) OVER (PARTITION BY instance, run_ms, month), 0) AS share,
         max(value) OVER (PARTITION BY instance, run_ms, month) - value AS gap
  FROM company_month
),
d AS (
  SELECT a.instance, a.run_ms, a.company, a.mi AS start_mi, b.share - a.share AS delta
  FROM f a JOIN f b ON b.instance = a.instance AND b.run_ms = a.run_ms AND b.company = a.company
                   AND b.mi = a.mi + $horizon
),
span AS (
  SELECT instance, run_ms, max(mi) AS last_mi FROM f GROUP BY ALL
),
latest AS (
  SELECT max(run_ms) AS run_ms FROM run_spans
),
act AS (
  SELECT *, CAST(substr(month, 1, 4) AS BIGINT) * 12 + CAST(substr(month, 6, 2) AS BIGINT) - 1 AS state_mi,
         CASE service WHEN 'watchdog' THEN 1.0
                      WHEN 'build_audit' THEN TRY_CAST(json_extract_string(detail, '$.defects_total') AS DOUBLE)
                      WHEN 'gap_watch' THEN TRY_CAST(json_extract_string(detail, '$.level') AS DOUBLE) END AS severity
  FROM service_actions
  QUALIFY row_number() OVER (PARTITION BY instance, run_ms, service, action, company, month ORDER BY seq) = 1
),
w AS (
  SELECT instance, run_ms, seq, game_date, json_extract_string(data, '$.key') AS key,
         CASE split_part(json_extract_string(data, '$.key'), '.', 1)
           WHEN 'watchdog' THEN CASE WHEN json_extract_string(data, '$.value.stalled') = 'true' THEN 1.0 ELSE 0.0 END
           WHEN 'audit' THEN TRY_CAST(json_extract_string(data, '$.value.defects_total') AS DOUBLE)
           WHEN 'gap' THEN TRY_CAST(json_extract_string(data, '$.value.level') AS DOUBLE) END AS severity
  FROM run_events
  WHERE kind = 'rhai.shared' AND json_extract_string(data, '$.op') = 'set'
    AND regexp_full_match(coalesce(json_extract_string(data, '$.key'), ''), '(watchdog|audit|gap)\.[0-9]+')
),
res AS (
  SELECT act.id, min(w.game_date) AS resolved_on
  FROM act JOIN w ON w.instance = act.instance AND w.run_ms = act.run_ms AND w.key = act.key
                 AND w.seq > act.seq AND w.severity < act.severity
                 AND CAST(substr(w.game_date, 1, 4) AS BIGINT) * 12 + CAST(substr(w.game_date, 6, 2) AS BIGINT)
                     <= act.state_mi + $horizon
  GROUP BY act.id
),
base AS (
  SELECT act.id, median(d.delta) AS baseline, count(d.delta) AS baseline_windows
  FROM act JOIN d ON d.instance = act.instance AND d.run_ms = act.run_ms AND d.company = act.company
                 AND d.start_mi + $horizon <= act.state_mi
  GROUP BY act.id
)
SELECT act.id, act.instance, act.run_ms, act.seed, act.service, act.action, act.company, act.month,
       act.game_date, act.expected, act.share AS share_at, f1.share AS share_after,
       f1.share - act.share AS delta_share, base.baseline, base.baseline_windows,
       f1.share - act.share - base.baseline AS relative_outcome,
       act.value AS value_at, f1.value AS value_after, f1.value - act.value AS value_change,
       act.gap AS gap_at, f1.gap AS gap_after,
       CASE WHEN act.severity IS NULL THEN NULL WHEN res.resolved_on IS NOT NULL THEN true
            WHEN f1.share IS NULL THEN NULL ELSE false END AS resolved,
       date_diff('day', CAST(act.game_date AS DATE), CAST(res.resolved_on AS DATE)) AS resolved_days,
       CASE WHEN act.share IS NULL THEN 'no_state'
            WHEN f1.share IS NOT NULL AND base.baseline IS NULL THEN 'no_baseline'
            WHEN f1.share IS NOT NULL THEN 'scored'
            WHEN span.last_mi >= act.state_mi + $horizon THEN 'company_gone'
            WHEN act.run_ms = (SELECT run_ms FROM latest) THEN 'pending'
            ELSE 'ended' END AS status
FROM act
LEFT JOIN f f1 ON f1.instance = act.instance AND f1.run_ms = act.run_ms AND f1.company = act.company
              AND f1.mi = act.state_mi + $horizon
LEFT JOIN base ON base.id = act.id
LEFT JOIN res ON res.id = act.id
LEFT JOIN span ON span.instance = act.instance AND span.run_ms = act.run_ms
ORDER BY act.run_ms, act.seq;
