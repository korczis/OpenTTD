-- One JSON field of every event of a kind, as text: what a one-time import reads to skip the
-- records it holds already (src/panel_store.rs, src/duel_outcomes.rs). Parameters: $kind (e.g.
-- plan.duel), $path (a JSON path, e.g. $.inputs.live.record).
SELECT json_extract_string(data, CAST($path AS VARCHAR)) AS value FROM events WHERE kind = $kind;
