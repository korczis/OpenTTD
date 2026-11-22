-- One bus envelope as a row (Store::insert_events, src/bus_sinks.rs): the instance it is about,
-- its sequence number, time, kind, game clock and payload.
INSERT INTO events (instance, seq, ts_ms, kind, game_tick, game_date, data)
VALUES ($instance, $seq, $ts_ms, $kind, $game_tick, $game_date, $data);
