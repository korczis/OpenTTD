-- The editions of the battle's store, newest first (by the latest match start).
SELECT edition FROM matches GROUP BY edition ORDER BY max(started_at) DESC;
