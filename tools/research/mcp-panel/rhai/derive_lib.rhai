// Thresholds derived from the panel's store instead of typed by hand (RESEARCH-ONLY;
// research/mechanisms/service-self-evaluation.md "Derived thresholds"). A service imports it:
//   import "derive_lib" as dl;
//   let t = dl::pick(dl::baselines(6), "quiet_days", "p90", 1.5, START_DAYS, 20, 7, 365);
//   t.value   the threshold; t.source "derived" or "fallback"; t.n, t.runs, t.why say from what.
// The numbers come from the named query baselines (tools/research/mcp-rhai/sql/queries/baselines.sql),
// read from the runs before the store's latest one only, so a running game never calibrates on
// itself. The hand-typed constant a caller passes is the cold-start fallback, used while the store
// holds fewer than min_n values of the metric or cannot answer at all; the answer says which.

// The rows of baselines as maps (metric, n, runs, p50, p75, p90); `months` is the window of
// leader_ratio_drop. () when the store or the panel cannot answer (no store, no panel, a test) or
// answers what is not a table of rows: it never throws, so no caller's round is lost to it.
fn baselines(months) {
    let r = ();
    try { r = api_dbNamedRun(#{ name: "baselines", params: #{ months: months } }); } catch { return (); }
    if type_of(r) != "map" || "error" in r || "http_error" in r { return (); }
    let out = [];
    try {
        let cols = r.columns ?? [];
        for row in r.rows ?? [] {
            let m = #{};
            for i in 0..cols.len() { m[cols[i]] = row[i]; }
            out.push(m);
        }
    } catch { return (); }
    out
}

// A threshold from the baselines rows: `factor` times the metric's percentile `pct` ("p50", "p75" or
// "p90"), kept within lo..hi, when the rows hold at least `min_n` values of it; else `fallback`, the
// caller's declared cold-start constant.
fn pick(rows, metric, pct, factor, fallback, min_n, lo, hi) {
    let out = #{ metric: metric, percentile: pct, factor: factor, n: 0, runs: 0, min_n: min_n,
                 fallback: fallback, source: "fallback", value: fallback, query: "baselines" };
    if type_of(rows) != "array" { out.why = "the store did not answer"; return out; }
    for r in rows {
        if r.metric != metric { continue; }
        out.n = r.n;
        out.runs = r.runs;
        let p = r[pct];
        if r.n >= min_n && type_of(p) != "()" {
            let v = p * factor;
            out.value = if v < lo { lo } else if v > hi { hi } else { v };
            out.source = "derived";
            out.why = `${factor} x ${pct} ${p} of ${r.n} value(s) over ${r.runs} earlier run(s), within ${lo}..${hi}`;
        }
    }
    if out.source == "fallback" { out.why = `${out.n} value(s) in the store, fewer than ${min_n}: the declared fallback ${fallback}`; }
    out
}
