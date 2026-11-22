-- The final standings of every match of an edition, by match and rank. Parameter: $edition.
SELECT match_id, contestant, value, vehicles, stations
FROM standings
WHERE edition = $edition
ORDER BY match_id, rank;
