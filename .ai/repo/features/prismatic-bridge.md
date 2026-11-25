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

**draft** (definition: `README.md#what-a-status-means`). Criterion 1 is not met: the feature is
not complete on master. The bridge itself is a sibling repository
(`~/dev/openttd-prismatic-bridge`, Python/FastAPI), and the native Rust client that ADR 0032
decides has not been built. Phase 3 is Partial in the phase table of
`research/prismatic-bridge/ARCHITECTURE.md`. Prismatic's own decision is not yet run live, with
its effect read back from the game (I0168, milestone `prismatic-closes-one-real-loop`).
Criterion 2 is met only by hand: `tools/research/bridge-live-check` needs the sibling bridge
checkout, and neither CI nor a gate tier runs it. Its last recorded PASS was on 2026-11-16 at
f5ffdf3b17 (I0025).
