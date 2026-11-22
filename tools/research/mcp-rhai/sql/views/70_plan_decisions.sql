-- Every planning decision (plan.* events): its kind, decision, choice and version.
CREATE OR REPLACE VIEW plan_decisions AS
  SELECT instance, run_ms, seed, seq, game_date, kind, data->>'decision' AS decision,
  data->'choice' AS choice, data->'version' AS version
  FROM run_events WHERE kind LIKE 'plan.%';

COMMENT ON VIEW plan_decisions IS 'Every planning decision (plan.* events): its kind, decision, choice and version.';
COMMENT ON COLUMN plan_decisions.instance IS 'the instance that decided (run_events.instance)';
COMMENT ON COLUMN plan_decisions.run_ms IS 'the start of the run, Unix milliseconds (run_spans.run_ms)';
COMMENT ON COLUMN plan_decisions.seed IS 'the run''s map seed, or null (run_spans.seed)';
COMMENT ON COLUMN plan_decisions.seq IS 'the plan.* event''s sequence number, increasing within a run';
COMMENT ON COLUMN plan_decisions.game_date IS 'the game''s date YYYY-MM-DD when the event was published, or null';
COMMENT ON COLUMN plan_decisions.kind IS 'the event kind, plan.<what>, e.g. plan.portfolio, plan.bot_settings, plan.directive';
COMMENT ON COLUMN plan_decisions.decision IS 'the name of the decision (data->>''decision''), e.g. bot.portfolio, bot.settings';
COMMENT ON COLUMN plan_decisions.choice IS 'what the decision chose, as JSON (data->''choice'')';
COMMENT ON COLUMN plan_decisions.version IS 'what decided, as JSON {revision, params, digest} (data->''version''): the git revision the binaries were built from (MCP_RHAI_REVISION), the decision''s parameters and their digest';
