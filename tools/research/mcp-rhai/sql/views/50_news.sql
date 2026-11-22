-- The news the owner's window showed (news.shown), one column per field: type, style, text, and advice and refs when the game reports them.
CREATE OR REPLACE VIEW news AS
  SELECT instance, run_ms, seed, seq, game_date, data->>'type' AS type, data->>'style' AS style,
  data->>'text' AS text, data->>'advice' AS advice, data->'refs' AS refs
  FROM run_events WHERE kind = 'news.shown';

COMMENT ON VIEW news IS 'The news the owner''s window showed (news.shown), one column per field: type, style, text, and advice and refs when the game reports them.';
COMMENT ON COLUMN news.instance IS 'the instance whose window showed the item (run_events.instance)';
COMMENT ON COLUMN news.run_ms IS 'the start of the run, Unix milliseconds (run_spans.run_ms)';
COMMENT ON COLUMN news.seed IS 'the run''s map seed, or null (run_spans.seed)';
COMMENT ON COLUMN news.seq IS 'the news.shown event''s sequence number, increasing within a run';
COMMENT ON COLUMN news.game_date IS 'the game''s date YYYY-MM-DD when the event was published, or null';
COMMENT ON COLUMN news.type IS 'the news type (data->>''type''), e.g. company_info, arrival_company, subsidies, advice';
COMMENT ON COLUMN news.style IS 'the news window''s style (data->>''style''): normal, thin, small or company';
COMMENT ON COLUMN news.text IS 'the item''s text as the window showed it (data->>''text'')';
COMMENT ON COLUMN news.advice IS 'for an advice item its advice type (data->>''advice''), e.g. vehicle_lost, train_stuck, vehicle_unprofitable, vehicle_old, order; else null';
COMMENT ON COLUMN news.refs IS 'the objects the item names as JSON (data->''refs''), e.g. [{vehicle, unit, owner}], {station}, {tile, x, y}, {industry}, {town}, {engine}; null when it names none';
