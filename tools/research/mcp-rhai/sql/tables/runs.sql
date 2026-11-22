-- One row per start of a publisher (a panel or the hub) on this file.
CREATE TABLE IF NOT EXISTS runs (
  instance VARCHAR,
  session VARCHAR,
  seed BIGINT,
  started_ms BIGINT,
  panel_port INTEGER,
  mcp_port INTEGER,
  game_port INTEGER
);

COMMENT ON TABLE runs IS 'One row per start of a publisher (a panel or the hub) on this file.';
COMMENT ON COLUMN runs.instance IS 'the publisher, e.g. panel-8800 or hub-8790';
COMMENT ON COLUMN runs.session IS 'the Claude or Majordomus session that started it, or null';
COMMENT ON COLUMN runs.seed IS 'the game''s map seed, or null for a publisher without a game';
COMMENT ON COLUMN runs.started_ms IS 'when it started, Unix milliseconds; run_spans calls it run_ms';
COMMENT ON COLUMN runs.panel_port IS 'the panel''s HTTP port, or null';
COMMENT ON COLUMN runs.mcp_port IS 'the game''s MCP port, or null';
COMMENT ON COLUMN runs.game_port IS 'the game''s network port, or null';
