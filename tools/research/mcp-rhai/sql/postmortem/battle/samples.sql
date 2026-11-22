-- Every sample of every contestant of an edition, every column by name, so a column the referee
-- adds later is carried too (the store is add-only). Parameter: $edition.
SELECT * FROM samples WHERE edition = $edition ORDER BY match_id, contestant, t;
