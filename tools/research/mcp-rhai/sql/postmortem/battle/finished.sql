-- How many original matches (not replicas) of an edition are done. Parameter: $edition.
SELECT count(*) FROM matches WHERE edition = $edition AND status = 'done' AND replica_of IS NULL;
