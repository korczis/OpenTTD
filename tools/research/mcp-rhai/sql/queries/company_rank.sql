-- A company's rank by value in the current run, with how many companies there are, its value and
-- the date of its latest sample. Parameter: $company (0-based id).
SELECT rank, (SELECT count(*) FROM company_latest l2 WHERE l2.run_ms = l.run_ms) AS of, value, game_date
FROM company_latest l
WHERE run_ms = (SELECT max(run_ms) FROM run_spans) AND company = $company;
