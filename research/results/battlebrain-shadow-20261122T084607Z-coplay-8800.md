# battlebrain-shadow-20261122T084607Z-coplay-8800

Derived from its .json (the shadow run's own output plus provenance); do not edit by hand.

RESEARCH-ONLY. **Shadow mode:** the decision was never submitted, and its directive block says `mode: shadow`, which the executor refuses (`panel_decision.rs` `decision_gate`). The run holds for this one capture, this company and these parameters, and does not generalise.

- **Capture:** the owner's co-play game on :8800, one read-only pass, ticks 85325-85368. The capture is **not coherent**, because the game was running while it was read.
- **Policy:** `freight-margin@1`, margin 0.2, reserve 25,000.
- **Company:** 3, with a budget of 125,011 (money plus loan headroom minus the reserve).
- **Portfolio solver:** AlgoGlyph `knapsack-dp`, at commit `1b8e009059`.
- **Reproducibility:** each replay of the capture, and the shadow run on it, ran twice and gave byte-identical output.

| | line | investment | monthly profit | annual return | payback (months) | planner score | track |
|---|---|---|---|---|---|---|---|
| planner v4 (baseline) | 41 → 1 (cargo 1) | 127,603 | 19,431 | 1.8273 | 6.567 | 338,752 | found |
| battlebrain | 41 → 46 (cargo 1) | 88,302 | 13,730 | 1.8659 | 6.4313 | 241,230 | not priced |

**Divergence:** `input_model_discrepancy`. the planner's line fails freight-margin@1 on the same snapshot: investment 127603 exceeds what the company can spend and keep the reserve (125011). The planner prices its portfolio against `max_loan` (300,000). The company can spend less than that without breaking its reserve.

**Finding for cutover:** battlebrain's line has no track-planning price. On this capture 24 lines are unpriced and 3 cannot be routed. Before a live decision, the selected line must carry `track: found` (I0096).

The other lines: 39 were rejected, each for its reason (see the .json).
