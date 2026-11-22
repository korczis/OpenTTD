-- A company's last months in the current run, newest first: value, its change from the month
-- before, money and its change, vehicles. Parameters: $company (0-based id), $months (how many).
SELECT month, value, value_change, money, money_change, vehicles
FROM company_month
WHERE run_ms = (SELECT max(run_ms) FROM run_spans) AND company = $company
ORDER BY month DESC
LIMIT $months;
