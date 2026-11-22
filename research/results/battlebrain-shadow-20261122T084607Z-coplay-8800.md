# battlebrain-shadow-20261122T084607Z-coplay-8800

Derived from its .json by `battlebrain report` (tools/research/mcp-rhai/src/bin/battlebrain.rs); do not edit.

RESEARCH-ONLY. Mode **shadow**: the decision was never submitted; its directive block says `mode: shadow`, which the executor refuses (`panel_decision.rs` `decision_gate`). One capture, one company and these parameters only; it does not generalise.

- **Capture:** the owner's 8-company co-play game (feature/research-coplay), ticks 85325-85368: **not coherent**: the game ran while it was read.
- **Policy:** `freight-margin@1` with {"margin":0.2,"reserve":25000}; company 3; budget 125,011 (money + loan headroom - reserve).
- **Portfolio solver:** algoglyph knapsack-dp; AlgoGlyph commit `1b8e009059`.
- **Lines:** 24 unpriced, 3 unroutable, 39 rejected with their reasons (see the .json).

| | line | investment | monthly profit | annual return | payback (months) | planner score | track |
|---|---|---|---|---|---|---|---|
| planner v4 (baseline) | 41 → 1 (cargo 1) | 127,603 | 19,431 | 1.8273 | 6.567 | 338,752 | found |
| battlebrain | 41 → 46 (cargo 1) | 88,302 | 13,730 | 1.8659 | 6.4313 | 241,230 | not priced |

**Divergence:** `input_model_discrepancy`. the planner's line fails freight-margin@1 on the same snapshot: investment 127603 exceeds what the company can spend and keep the reserve (125011).

**Finding:** battlebrain's line has no track-planning price; freight-margin@2 (I0097) adopts only priced lines, and the cutover gate requires `track: found`.
