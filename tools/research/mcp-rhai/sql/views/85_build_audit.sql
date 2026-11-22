-- Each company's construction audit of each game month of each run: the last audit.<company> the build_audit.rhai service wrote to the panel's shared memory that month (rhai.shared events), one column per count.
CREATE OR REPLACE VIEW build_audit AS
  SELECT instance, run_ms, seed, company, month, game_date,
  road_depots, rail_depots, ship_depots, open_depots, dead_end_depots, no_exit_depots,
  duplicate_depots, duplicate_stations, drive_through_stops, bay_stops, drive_through_share,
  defects, new_defects
  FROM (SELECT instance, run_ms, seed, seq,
  CAST(data->'value'->>'company' AS BIGINT) AS company,
  data->'value'->>'month' AS month, data->'value'->>'date' AS game_date,
  CAST(data->'value'->'depots'->>'road' AS BIGINT) AS road_depots,
  CAST(data->'value'->'depots'->>'rail' AS BIGINT) AS rail_depots,
  CAST(data->'value'->'depots'->>'ship' AS BIGINT) AS ship_depots,
  CAST(data->'value'->'exits'->>'open' AS BIGINT) AS open_depots,
  CAST(data->'value'->'exits'->>'dead_end' AS BIGINT) AS dead_end_depots,
  CAST(data->'value'->'exits'->>'no_exit' AS BIGINT) AS no_exit_depots,
  CAST(data->'value'->>'duplicate_depots' AS BIGINT) AS duplicate_depots,
  CAST(data->'value'->>'duplicate_stations' AS BIGINT) AS duplicate_stations,
  CAST(data->'value'->'stops'->>'drive_through' AS BIGINT) AS drive_through_stops,
  CAST(data->'value'->'stops'->>'bay' AS BIGINT) AS bay_stops,
  CAST(data->'value'->'stops'->>'drive_through_share' AS DOUBLE) AS drive_through_share,
  CAST(data->'value'->>'defects_total' AS BIGINT) AS defects,
  CAST(json_array_length(data->'value'->'new_keys') AS BIGINT) AS new_defects
  FROM run_events
  WHERE kind = 'rhai.shared' AND (data->>'op') = 'set' AND regexp_full_match(data->>'key', 'audit\.[0-9]+'))
  QUALIFY row_number() OVER (PARTITION BY instance, run_ms, company, month ORDER BY seq DESC) = 1;

COMMENT ON VIEW build_audit IS 'Each company''s construction audit of each game month of each run: the last audit.<company> the build_audit.rhai service wrote to the panel''s shared memory that month (rhai.shared events), one column per count.';
COMMENT ON COLUMN build_audit.instance IS 'the panel whose memory held the audit (run_events.instance)';
COMMENT ON COLUMN build_audit.run_ms IS 'the start of the run, Unix milliseconds (run_spans.run_ms)';
COMMENT ON COLUMN build_audit.seed IS 'the run''s map seed, or null (run_spans.seed)';
COMMENT ON COLUMN build_audit.company IS 'the company id, 0-based (company 1 in the game is 0); its shared key is audit.<company>';
COMMENT ON COLUMN build_audit.month IS 'the game month YYYY-MM the audit judged';
COMMENT ON COLUMN build_audit.game_date IS 'the game date YYYY-MM-DD of that audit';
COMMENT ON COLUMN build_audit.road_depots IS 'the company''s road depots (openttd://depots)';
COMMENT ON COLUMN build_audit.rail_depots IS 'the company''s rail depots';
COMMENT ON COLUMN build_audit.ship_depots IS 'the company''s ship depots';
COMMENT ON COLUMN build_audit.open_depots IS 'depots a vehicle leaves onto its entrance and on from there (openttd://map/ways)';
COMMENT ON COLUMN build_audit.dead_end_depots IS 'depots whose entrance leads into the depot and nowhere else';
COMMENT ON COLUMN build_audit.no_exit_depots IS 'depots whose entrance tile does not lead into the depot at all';
COMMENT ON COLUMN build_audit.duplicate_depots IS 'pairs of the company''s depots of one type within the game''s service reach of each other (by road when the ways are known)';
COMMENT ON COLUMN build_audit.duplicate_stations IS 'pairs of the company''s stations sharing a facility with overlapping catchments (the hub.audit rule)';
COMMENT ON COLUMN build_audit.drive_through_stops IS 'road stop tiles of the company that are drive-through';
COMMENT ON COLUMN build_audit.bay_stops IS 'road stop tiles of the company that are bays';
COMMENT ON COLUMN build_audit.drive_through_share IS 'drive_through_stops / (drive_through_stops + bay_stops), null without road stops or without the map';
COMMENT ON COLUMN build_audit.defects IS 'every defect the audit found: depots without an exit, duplicate depots and stations, bay stops';
COMMENT ON COLUMN build_audit.new_defects IS 'the defects that were not in the company''s audit before; 0 in the baseline round';
