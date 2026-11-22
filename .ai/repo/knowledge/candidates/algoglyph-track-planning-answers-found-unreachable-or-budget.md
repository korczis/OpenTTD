---
schema: knowledge/v1
id: algoglyph-track-planning-answers-found-unreachable-or-budget
kind: knowledge
class: fact
title: "AlgoGlyph's track-planning kernel answers found, unreachable or budget, never ok; only found prices a line"
description: "Before reading a track-planning answer, or writing a check, fixture or policy that tests its status."
status: candidate
epistemics: observed
date: 2026-11-22
tags: [battlebrain, algoglyph, track-planning, contract]
about: tools/research/mcp-rhai/src/battlebrain_contract.rs
route: project
provenance:
  origin: authored
  derived_from:
    - file:tools/research/mcp-rhai/src/battlebrain_contract.rs
    - file:tools/research/mcp-rhai/src/battlebrain_policy.rs
    - commit:194552a5fa011bfc1c221f3e4c4e77979cb6e69c
---

# Fact

AlgoGlyph's kernel types its answer as `status: 'found' | 'unreachable' | 'budget'`
(`algoglyph:src/plates/track-planning/plan.ts:127` at `origin/HEAD` 30a56ef; `found` is returned
at :363, the other two through `empty()` at :366). There is no `ok`. A `found` plan carries a
finite cost and its pieces; `unreachable` and `budget` carry cost `Infinity` and no pieces.

At 194552a5fa the fork checks it in one place: `battlebrain_contract.rs:685-692` (`check_track`,
registered as the track-planning adapter's `check` at :655 and run by `domain_check` at :710)
refuses any status other than `found`, and the test at :1230-1236 refuses `unreachable`,
`budget` and `ok` alike. `battlebrain_policy.rs:124` treats a non-`found` track as "did not find a
route".

# How to apply

Test `status == "found"` before using a plan's cost or money; never `ok`, and never a cost alone
(an unpriced plan's cost is not finite).

# Review

Authored 2026-11-22 from a read of origin/master at 194552a5fa and algoglyph at 30a56ef;
awaiting the owner's review.
`majordomus knowledge promote algoglyph-track-planning-answers-found-unreachable-or-budget` with
evidence, or `majordomus knowledge reject algoglyph-track-planning-answers-found-unreachable-or-budget --reason ...`.
