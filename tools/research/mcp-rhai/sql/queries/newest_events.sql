-- The newest 20 events: what GET /api/db/query answers without ?sql=, so the bare address shows
-- what the store holds.
SELECT seq, ts_ms, kind, game_tick, game_date, data FROM events ORDER BY ts_ms DESC, seq DESC LIMIT 20;
