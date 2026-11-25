---
schema: feature/v1
id: prismatic-bridge
kind: feature
title: Prismatic bridge steering a Company AI
short_title: Bridge
headline: Prismatic decides, a standalone bridge relays over the admin network, and a normal Company AI acts in the game.
summary: The bridge (a sibling Python/FastAPI service, to be rewritten in Rust per ADR 0032) drives PrismaticAI (tools/research/prismatic-ai, moved here from the bridge's ai/ on 2026-11-18) through rcon start_ai and, on this fork's research build, sets a running AI's settings through openttd.ai.set_setting; prismatic-platform answers the decision contract, and the MCP panel consumes it without the bridge.
status: draft
weight: 40
areas: [agent-integration]
audiences: [prismatic-integrator]
rules: [project.game-state-mutates-only-through-commands, project.a-tuning-claim-needs-held-start-pairs]
docs: [research/prismatic-bridge/ARCHITECTURE.md, docs/admin_network.md]
adrs: [adr-0007, adr-0032, adr-0034]
related: [embedded-mcp-server, event-bus-and-store]
tags: [bridge, prismatic, admin-network]
---

## What it does

Phases 1 and 2 work: the control round trip, and a real single-town bus route. The decision
contract behind `/decision/pull` is a JSON Schema with fixtures in this repository (I0011);
`prismatic-platform` answers it at `GET /api/openttd/decision` (a Monte Carlo choice of
`aggressiveness`), and the MCP panel pulls and applies it (`POST /api/decision/pull`). A directive
with a `company_id` sets the decided settings on the running AI through the approval-gated
`openttd.ai.set_setting`, so it no longer needs an AI restart on a research build; the bridge does
the same.

## What it does not do

Prismatic still decides from one read of the companies and a hand growth model, not from recorded
history (ADR 0034). This fork's half is done (I0085): the panel offers each company's history (the
bus endpoint and a store query of its samples) and records the fit a directive reports, or states
the fallback; fitting it is `prismatic-platform`'s own change. The Rust rewrite of the bridge client (ADR 0032, accepted) is not done. On a
vanilla server the only control is still rcon `start_ai`/`reload_ai`, which restarts the AI.

## Maturity

`draft`, graded 2026-11-25 against [the levels](README.md#maturity-levels).

- **missing**: `shipped` and `tested` for the relay the headline names. The bridge is a sibling
  repository (`research/prismatic-bridge/ARCHITECTURE.md`), so nothing on this master ships or
  tests it. Its end-to-end check, `tools/research/bridge-live-check`, is run by hand only, and the
  Rust rewrite decided in I0012 is not done. This repository's half is in place: the decision
  contract `research/prismatic-bridge/decision-contract.schema.json` and the panel's pull
  (`tools/research/mcp-rhai/src/panel_decision.rs:782:pull_with`), tested by
  `tools/research/mcp-rhai/src/panel_decision.rs:1176:the_contracts_example_and_prismatics_answer_are_directives`
  and gated in CI (`.github/workflows/research-rust.yml:190-191:cargo`).
- **documented**: `research/prismatic-bridge/ARCHITECTURE.md`.
