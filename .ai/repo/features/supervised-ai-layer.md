---
schema: feature/v1
id: supervised-ai-layer
kind: feature
title: Supervised AI layer derived from the fork's history
short_title: AI layer
headline: Decisions, rules, plan, intents and knowledge live in tracked records that every agent and provider reads the same way.
summary: Majordomus supervises the checkout; .ai/ holds the ADRs, project rules, curated knowledge, milestones, issues and intents, all cross-linked and traced to commits by Refs trailers.
status: stable
weight: 70
areas: [validation, upstream-currency]
kinds: [adr, rule, knowledge, milestone, issue, intent]
audiences: [research-owner, coding-agent]
rules: [project.no-upstream-interaction, project.no-commit-without-current-instruction, project.personal-tooling-stays-uncommitted]
docs: [AGENTS.md]
adrs: [adr-0010, adr-0011]
related: [layered-validation-gate]
tags: [majordomus, governance, traceability]
---

## What it does

Every commit runs `majordomus doctor`, every push runs `majordomus finish --check`, and the
plan's state is derived from the recorded stamps and evidence.

## What it does not do

It does not accept a proposed ADR. That is the owner's act.

## Maturity

`stable`, graded 2026-11-25 against [the levels](README.md#maturity-levels).

- **shipped**: the layer's manifest (`.ai/manifest.yaml:3:schema`) and its policy's enforcement
  entries (`.ai/repo/policy.yaml:195:enforcement`).
- **tested**: `majordomus doctor` proves the layer: the manifest resolves, the rules resolve, and
  every enforcement entry is wired (`.ai/repo/policy.yaml:196:doctor`).
- **gated**: the pre-commit hook runs `majordomus doctor` and the pre-push hook runs
  `majordomus finish --check` (`.ai/repo/policy.yaml:196:doctor`, `.ai/repo/policy.yaml:200:finish`).
- **documented**: `AGENTS.md`.
