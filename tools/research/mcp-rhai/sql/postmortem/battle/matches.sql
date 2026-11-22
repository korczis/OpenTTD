-- Every match of an edition: its host, seed, status, the match it replicates and its lineup (JSON).
-- Parameter: $edition.
SELECT id, host, seed, status, replica_of, lineup FROM matches WHERE edition = $edition;
