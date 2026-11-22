-- One start of a publisher on this file (Store::insert_run, src/bus_sinks.rs); a seed or port the
-- publisher does not have is null.
INSERT INTO runs (instance, session, seed, started_ms, panel_port, mcp_port, game_port)
VALUES ($instance, $session, $seed, $started_ms, $panel_port, $mcp_port, $game_port);
