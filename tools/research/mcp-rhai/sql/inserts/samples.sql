-- One company of one game.sample (Store::insert_samples, src/bus_sinks.rs): its money, loan, value,
-- net and fleet at that moment.
INSERT INTO samples (instance, ts_ms, game_tick, game_date, company, name, money, loan, value, net, vehicles)
VALUES ($instance, $ts_ms, $game_tick, $game_date, $company, $name, $money, $loan, $value, $net, $vehicles);
