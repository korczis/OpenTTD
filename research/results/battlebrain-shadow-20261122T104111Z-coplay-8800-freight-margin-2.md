# battlebrain-shadow-20261122T104111Z-coplay-8800-freight-margin-2

Derived from its .json by `battlebrain report` (tools/research/mcp-rhai/src/bin/battlebrain.rs); do not edit.

RESEARCH-ONLY. Mode **shadow**: the decision was never submitted; its directive block says `mode: shadow`, which the executor refuses (`panel_decision.rs` `decision_gate`). One capture, one company and these parameters only; it does not generalise.

- **Capture:** the owner's 8-company co-play game (feature/research-coplay), ticks 336603-336631: **not coherent**: the game ran while it was read.
- **Policy:** `freight-margin@2` with {"margin":0.2,"require_priced":true,"reserve":25000}; company 3; budget 130,297 (money + loan headroom - reserve).
- **Portfolio solver:** algoglyph knapsack-dp; AlgoGlyph commit `1b8e009059`.
- **Lines:** 24 unpriced, 6 unroutable, 39 rejected with their reasons (see the .json).

| | line | investment | monthly profit | annual return | payback (months) | planner score | track |
|---|---|---|---|---|---|---|---|
| planner v4 (baseline) | 29 → 34 (cargo 7) | 113,279 | 19,139 | 2.0275 | 5.9188 | 346,056 | not priced |
| battlebrain | 41 → 1 (cargo 1) | 125,241 | 22,034 | 2.1112 | 5.684 | 403,573 | found |

**Divergence:** `input_model_discrepancy`. the planner's line fails freight-margin@2 on the same snapshot: track-planning did not price this line on the map.

**Finding:** the planner's own choice was a line track-planning never priced (it priced 16 of 40 candidates)
