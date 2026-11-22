-- The post-mortem's tables in analytics.duckdb (research/battle-postmortem.md): every edition's
-- analyses, contestants and proposals, created when missing.
CREATE TABLE IF NOT EXISTS analyses (
  edition VARCHAR, match_id VARCHAR, contestant VARCHAR, rank INTEGER, field INTEGER,
  last BOOLEAN, value DOUBLE, value_share_of_winner DOUBLE, peak_vehicles DOUBLE,
  final_vehicles DOUBLE, idle_cash_share DOUBLE, income_per_vehicle DOUBLE,
  stations_per_vehicle DOUBLE, gap_opens VARCHAR, causes VARCHAR[]
);
CREATE TABLE IF NOT EXISTS contestants (
  edition VARCHAR, contestant VARCHAR, prismatic BOOLEAN, settings VARCHAR, matches INTEGER,
  avg_rank DOUBLE, last_place INTEGER, avg_value_share_of_winner DOUBLE,
  avg_idle_cash_share DOUBLE, median_gap_opens VARCHAR, causes JSON
);
CREATE TABLE IF NOT EXISTS proposals (
  edition VARCHAR, contestant VARCHAR, settings VARCHAR, because JSON
);
-- The seed, so a contestant pairs with the default on the same map (the brain's paired target).
ALTER TABLE analyses ADD COLUMN IF NOT EXISTS seed BIGINT;
