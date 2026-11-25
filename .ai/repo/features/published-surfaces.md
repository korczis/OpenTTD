---
schema: feature/v1
id: published-surfaces
kind: feature
title: Published surfaces derived from the repository
short_title: Surfaces
headline: The site, the wiki and the Markdown index tables, all generated from what git tracks and the .ai/ layer, and checked on every commit.
summary: tools/surfaces renders every tracked Markdown document to a GitHub Pages site and the GitHub wiki, keeps generated regions inside the documents current, and fails the commit on a stale region or a dangling link.
status: stable
weight: 60
areas: [validation]
audiences: [research-owner]
docs: [README.md]
adrs: [adr-0012]
related: [supervised-ai-layer]
tags: [site, wiki, pages, documentation, derivation]
---

## What it does

- **Site.** `tools/surfaces/surfaces site` renders every tracked Markdown document as a
  page. It adds an overview with interactive ECharts views of commits, documentation and
  plan progress, an `.ai/` layer page with features, decisions, rules, the plan and a
  Mermaid dependency graph, and client-side search. `.github/workflows/surfaces.yml`
  deploys it to GitHub Pages from `master`.
- **Wiki.** `tools/surfaces/surfaces wiki` turns the same documents into wiki pages, and
  the workflow pushes them to the GitHub wiki.
- **Markdown regions.** `tools/surfaces/surfaces sync` regenerates the tables between
  `surfaces:begin` and `surfaces:end` markers inside the documents: indexes, facts, a table of
  contents from the headings that follow it, and a region of another document included with its
  links rewritten.
- **Generators.** The same `sync` first runs every other generator `surfaces.toml` declares (the
  README scoreboard, the MCP contributor docs, the town model), so one command updates every
  derived document.
- **Gate.** `tools/surfaces/surfaces check` fails on a stale region, on a declared generator
  whose check fails, or on a relative link or anchor to nothing. It runs in the pre-commit hook,
  in `tools/gate.sh change` and in CI.

## What it does not do

It publishes nothing a person writes for the site alone, and it does not edit upstream's
documents: those are exempted from the link check by path in `surfaces.toml`. The wiki is
overwritten on every sync, so an edit made there is lost.

## Maturity

`stable`, graded 2026-11-25 against [the levels](README.md#maturity-levels).

- **shipped**: `tools/surfaces/surfaces.py:2373:run_check` and the rest of `tools/surfaces/`.
- **tested**: the scratch-repository tests of `tools/surfaces/`, among them
  `tools/surfaces/test_facts_site_only.py:48:test_a_site_only_fact_is_not_written_into_the_region`
  and `tools/surfaces/test_maturity.py`.
- **gated**: the pre-commit hook (`.ai/repo/policy.yaml:206:surfaces`), `tools/gate.sh change`
  (`tools/gate.sh:634:check`) and CI, which runs both the check and the tests
  (`.github/workflows/surfaces.yml:64:check`, `.github/workflows/surfaces.yml:67:unittest`).
- **documented**: `README.md` and `tools/surfaces/README.md`. The second is not in `docs:`: no
  source class of `.ai/repo/knowledge/sources.yaml` indexes a README under `tools/`, so
  `majordomus product validate` refused the reference.
