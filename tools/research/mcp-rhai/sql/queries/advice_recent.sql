-- The newest advice news of the current run (a lost or stuck vehicle, one that loses money, ...),
-- with the objects each names. Parameter: $limit.
SELECT game_date, advice, text, refs
FROM advice
WHERE run_ms = (SELECT max(run_ms) FROM run_spans)
ORDER BY seq DESC
LIMIT $limit;
