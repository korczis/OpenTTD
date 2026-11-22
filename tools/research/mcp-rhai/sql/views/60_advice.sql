-- The news of type advice: a lost or stuck vehicle, one that loses money, an old one, a failed order or renewal.
CREATE OR REPLACE VIEW advice AS
  SELECT * FROM news WHERE type = 'advice';

COMMENT ON VIEW advice IS 'The news of type advice: a lost or stuck vehicle, one that loses money, an old one, a failed order or renewal.';
COMMENT ON COLUMN advice.instance IS 'the instance whose window showed the item (news.instance)';
COMMENT ON COLUMN advice.run_ms IS 'the start of the run, Unix milliseconds (run_spans.run_ms)';
COMMENT ON COLUMN advice.seed IS 'the run''s map seed, or null (run_spans.seed)';
COMMENT ON COLUMN advice.seq IS 'the news.shown event''s sequence number, increasing within a run';
COMMENT ON COLUMN advice.game_date IS 'the game''s date YYYY-MM-DD when the event was published, or null';
COMMENT ON COLUMN advice.type IS 'the news type, always advice here';
COMMENT ON COLUMN advice.style IS 'the news window''s style: normal, thin, small or company';
COMMENT ON COLUMN advice.text IS 'the item''s text as the window showed it';
COMMENT ON COLUMN advice.advice IS 'the advice type, e.g. vehicle_lost, train_stuck, vehicle_unprofitable, vehicle_old, order';
COMMENT ON COLUMN advice.refs IS 'the objects the item names as JSON, e.g. [{vehicle, unit, owner}]; null when it names none';
