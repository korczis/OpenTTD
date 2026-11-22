-- The news the owner's window showed (news.shown), one column per field: type, style, text, and advice and refs when the game reports them.
CREATE OR REPLACE VIEW news AS
  SELECT instance, run_ms, seed, seq, game_date, data->>'type' AS type, data->>'style' AS style,
  data->>'text' AS text, data->>'advice' AS advice, data->'refs' AS refs
  FROM run_events WHERE kind = 'news.shown';

COMMENT ON VIEW news IS 'The news the owner''s window showed (news.shown), one column per field: type, style, text, and advice and refs when the game reports them.';
