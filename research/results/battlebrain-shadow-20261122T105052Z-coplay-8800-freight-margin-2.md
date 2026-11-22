# battlebrain-shadow-20261122T105052Z-coplay-8800-freight-margin-2

Derived from its .json by `battlebrain report` (tools/research/mcp-rhai/src/bin/battlebrain.rs); do not edit.

RESEARCH-ONLY. Mode **shadow**: the decision was never submitted; its directive block says `mode: shadow`, which the executor refuses (`panel_decision.rs` `decision_gate`). One capture, one company and these parameters only; it does not generalise.

- **Capture:** the owner's 8-company co-play game (feature/research-coplay), ticks 356549-356588: **not coherent**: the game ran while it was read.
- **Policy:** `freight-margin@2` with {"margin":0.2,"require_priced":true,"reserve":25000}; company 3; budget 128,858 (money + loan headroom - reserve).
- **Portfolio solver:** algoglyph knapsack-dp; AlgoGlyph commit `1b8e009059`.
- **Lines:** 24 unpriced, 6 unroutable, 39 rejected with their reasons (see the .json).

| | line | investment | monthly profit | annual return | payback (months) | planner score | track |
|---|---|---|---|---|---|---|---|
| planner v4 (baseline) | 41 → 1 (cargo 1) | 125,331 | 22,034 | 2.1097 | 5.6881 | 403,483 | found |
| battlebrain | 41 → 1 (cargo 1) | 125,331 | 22,034 | 2.1097 | 5.6881 | 403,483 | found |

**Divergence:** `same`. both pick the same line.

