---
schema: knowledge/v1
id: planner-v4-budgets-against-max-loan-not-money-now
kind: knowledge
class: fact
title: "Planner v4 chooses its portfolio against difficulty.max_loan, not what the company can spend now; on the co-play game its pick was over budget or unpriced in two of three shadow runs"
description: "Before taking planner v4's chosen line as affordable or priced, or using it as the Battlebrain baseline."
status: candidate
epistemics: observed
date: 2026-11-22
tags: [battlebrain, planner, shadow, budget, track-planning]
about: tools/research/mcp-rhai/src/panel_bot.rs
route: project
provenance:
  origin: authored
  derived_from:
    - file:research/results/battlebrain-shadow-20261122T084607Z-coplay-8800-freight-margin-2.md
    - file:research/results/battlebrain-shadow-20261122T104111Z-coplay-8800-freight-margin-2.md
    - file:research/results/battlebrain-shadow-20261122T105052Z-coplay-8800-freight-margin-2.md
    - file:research/battlebrain/README.md
    - file:tools/research/mcp-rhai/src/panel_bot.rs
    - file:tools/research/mcp-rhai/src/battlebrain_policy.rs
relations:
  - type: relates_to
    target: file:.ai/repo/knowledge/curated/the-planner-overpredicts-what-a-line-earns.md
---

# Fact

The panel plans with `budget = difficulty.max_loan` (`panel_bot.rs:764`, :789-790 at 194552a5fa),
so the portfolio is sized to the loan ceiling, not to money plus loan headroom minus a reserve,
which is what Battlebrain's `freight-margin` policy spends (`battlebrain_policy.rs:14`, :114).

In the three `freight-margin@2` shadow runs on the co-play game (company 3, 2026-11-22):

- 1953-02-09 (`battlebrain-shadow-20261122T084607Z-...-freight-margin-2.md:14-17`): planner line
  41→1, investment 127,603 against a budget of 125,011 once the 25,000 reserve is kept;
- 1962-05-29 (`...T104111Z-...:14-19`): planner line 29→34, never priced by track-planning
  (16 of 40 candidates priced);
- 1963-02-22 (`...T105052Z-...:17`): both chose 41→1, priced.

`research/battlebrain/README.md:236-242` tabulates the three. None of the captures was coherent
(knowledge `a-planner-capture-is-coherent-only-through-read-many`).

# How to apply

Check a planner pick against money now minus the reserve and against `track: found` before
adopting or comparing it; three runs on one game are a small sample.

# Review

Authored 2026-11-22 from the result files and origin/master at 194552a5fa; awaiting the owner's
review. `majordomus knowledge promote planner-v4-budgets-against-max-loan-not-money-now`, or
`majordomus knowledge reject planner-v4-budgets-against-max-loan-not-money-now --reason ...`.
