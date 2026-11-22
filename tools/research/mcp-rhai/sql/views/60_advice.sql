-- The news of type advice: a lost or stuck vehicle, one that loses money, an old one, a failed order or renewal.
CREATE OR REPLACE VIEW advice AS
  SELECT * FROM news WHERE type = 'advice';

COMMENT ON VIEW advice IS 'The news of type advice: a lost or stuck vehicle, one that loses money, an old one, a failed order or renewal.';
