-- Each company's latest month of each run (company_month's columns) and its rank by value among that run's companies.
CREATE OR REPLACE VIEW company_latest AS
  SELECT *, rank() OVER (PARTITION BY instance, run_ms ORDER BY value DESC) AS rank
  FROM (SELECT * FROM company_month
  QUALIFY row_number() OVER (PARTITION BY instance, run_ms, company ORDER BY month DESC) = 1);

COMMENT ON VIEW company_latest IS 'Each company''s latest month of each run (company_month''s columns) and its rank by value among that run''s companies.';
