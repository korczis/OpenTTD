-- One row per company per game.sample: the company's money, loan, value and fleet at that moment.
CREATE TABLE IF NOT EXISTS samples (
  instance VARCHAR,
  ts_ms BIGINT,
  game_tick BIGINT,
  game_date VARCHAR,
  company BIGINT,
  name VARCHAR,
  money DOUBLE,
  loan DOUBLE,
  value DOUBLE,
  net DOUBLE,
  vehicles BIGINT
);

COMMENT ON TABLE samples IS 'One row per company per game.sample: the company''s money, loan, value and fleet at that moment.';
COMMENT ON COLUMN samples.instance IS 'the instance sampled';
COMMENT ON COLUMN samples.ts_ms IS 'when, Unix milliseconds';
COMMENT ON COLUMN samples.game_tick IS 'the game''s tick';
COMMENT ON COLUMN samples.game_date IS 'the game''s date YYYY-MM-DD';
COMMENT ON COLUMN samples.company IS 'the company id, 0-based (company 1 in the game is 0)';
COMMENT ON COLUMN samples.name IS 'the company''s name then';
COMMENT ON COLUMN samples.money IS 'cash, in the game''s internal currency';
COMMENT ON COLUMN samples.loan IS 'the loan';
COMMENT ON COLUMN samples.value IS 'the company value as the game computes it';
COMMENT ON COLUMN samples.net IS 'money minus loan';
COMMENT ON COLUMN samples.vehicles IS 'the number of vehicles';
