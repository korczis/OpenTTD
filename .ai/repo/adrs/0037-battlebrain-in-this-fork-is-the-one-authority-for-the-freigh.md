---
schema: adr/v1
id: adr-0037
kind: adr
title: Battlebrain in this fork is the one authority for the freight-line decision; planner and AlgoGlyph only produce candidates, the executor only applies
status: accepted
date: 2026-11-22
tags:
  - battlebrain
  - decision-authority
provenance:
  origin: authored
  derived_from:
    - issue:I0095
    - file:research/battlebrain/README.md
    - file:tools/research/mcp-rhai/src/battlebrain_contract.rs
---

# 37. Battlebrain in this fork is the one authority for the freight-line decision; planner and AlgoGlyph only produce candidates, the executor only applies

## Context

A read-only audit of 2026-11-22 found eleven components that choose an investment in a live
game. Among them: `plan.rs`, `cargo.rs` (a knapsack with a silent built-in fallback labelled as
the AlgoGlyph kernel), `linecost.rs`, `invest.rs`, `panel_bot.rs`, `intent.rs`, the Rhai
steward, PrismaticAI's own planners, Prismatic's aggressiveness Monte Carlo and
`prismatic_tune.rhai`. None of the freight lines handed to the bot carried a snapshot,
re-read or idempotency key. Kernels were called by hard-coded name with no version.

The owner chose the freight-line hybrid as the first decision type: every ~90 game days, the next
freight line or HOLD. The owner also ruled that prismatic-platform and algoglyph are used from
outside only. The canonical bridge is `feature/operator-approval`, and the blackboard is the
Majordomus mesh journal (owner, 2026-11-20).

## Decision

> Accepted by the owner on 2026-11-22 ("vse prijimam", then "vse xplicitne schvaluji, udelej to za me"). Majordomus allocated this record from the work it cites (its provenance `derived_from`); the owner adopted it as their decision, so its origin is now `authored`.

For the freight-line decision, `battlebrain` (`tools/research/mcp-rhai/src/bin/battlebrain.rs`)
is the only component that may write a decision record. The contract enforces this with
`writer_not_allowed`, in `battlebrain_contract.rs`.

- Planner v4 and the AlgoGlyph kernels produce candidate records. Each candidate rests on
  `algoglyph-invocation/v1` envelopes, whose kernel must be in the catalogue the runner reported
  at that checkout.
- The executor applies only a directive whose `battlebrain-decision/v1` block it can validate.
  The executor is the panel's `/api/decision/pull` or the bridge's operator-approval branch.
- The operator approves. Nothing approves its own request.
- The record, invocation and decision-block schemas are generated from that one Rust module.

## Alternatives rejected

- A policy module inside prismatic-platform. The owner does not want prismatic-platform edited.
- AlgoGlyph-side schemas and versions, for the same reason. The caller-side envelope pins the
  catalogue digest and checkout instead.
- Keeping planner v4 as the selector and adding provenance to it. That leaves eleven selectors.
- A new store for the records. The owner already chose the mesh journal.

## Consequences

- PrismaticAI's own freight pick must be switchable off for the governed company (a default-OFF
  research setting). Otherwise two components still choose a freight line. This is Phase 03.
- The built-in knapsack is a separately named solver and never passes as `knapsack-dp`. A kernel
  without a domain check cannot price or select a candidate.
- MCP resources must be read as one coherent snapshot before shadow mode can compare the planner
  with battlebrain on identical inputs.
- AlgoGlyph outputs stay unversioned, so a kernel's behaviour can change under an unchanged id.
  The envelope's catalogue digest and commit make such drift detectable, not impossible.
