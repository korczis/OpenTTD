---
schema: feature/v1
id: event-bus-and-store
kind: feature
title: Every panel event on a ZeroMQ socket and in a DuckDB store per instance
short_title: Event bus and store
headline: Any process on the machine follows a game live over ZeroMQ, and every decision can query what earlier games recorded.
summary: One tap on the panel's event bus forwards each event to a loopback ZeroMQ PUB socket (topic = kind, envelope prismatic-bus/v1) and to a DuckDB file per instance (runs, events, samples) with analytic views recreated at every open (run_spans, company_month, company_latest, run_events, news, advice, plan_decisions, kind_month, build_audit, schema_doc), read through /api/db/query, Rhai and the /data page; planning decisions publish their choices as plan.* events, and the calibration, the Prismatic directive's fit and the AlgoGlyph flow serve read the store.
status: stable
weight: 55
areas: [agent-integration, observability]
audiences: [prismatic-integrator, coding-agent]
rules: [project.mcp-surfaces-are-derived, project.a-tuning-claim-needs-held-start-pairs]
docs: [research/mcp-control.md]
adrs: [adr-0033, adr-0034]
related: [embedded-mcp-server, prismatic-bridge]
tags: [panel, events, zeromq, duckdb, planning]
---

## What it does

The bus's game, company, decision, console, news and panel events, plus the daily `game.sample`,
reach a ZeroMQ subscriber in bus order and reach the store within a second (I0080, ADR 0033).
`GET /api/bus`, `GET /api/db/tables` and the read-only `GET|POST /api/db/query` are routes of the
panel's table, so OpenAPI, Swagger UI and Rhai reach them too. The hub publishes the events it
raises itself through the same code (`tools/research/mcp-rhai/src/bus_sinks.rs`, I0084): its
`audit.*` findings and the `game.*` events it derives for a game without a panel go to its own
ZeroMQ socket and its own DuckDB store. A panel event the hub only relays stays in its panel's
store. The kinds and payloads are listed in knowledge `the-panel-bus-carries-these-event-kinds`.

These decisions read recorded history today:

- the planned bot's calibration (`PRISMATIC_PLAN_CALIBRATION` `all` or `seed`, off by default)
  reads every store's `plan.calibration` readings (I0082);
- the panel offers Prismatic each company's samples query and bus endpoint, and it records the fit
  that a directive reports as `plan.directive` (I0085);
- a long-lived AlgoGlyph flow serve is fed trailing rows of the store, and each call is published
  as `plan.kernel` with its live and stored inputs and their row counts
  (`tools/research/mcp-rhai/src/kernel_feed.rs`, I0086).

Every planning decision publishes its choice as a structured `plan.*` event beside its prose
(I0081). The kinds are one table, `tools/research/mcp-rhai/src/panel_events.rs`, listed by
`GET /api/bus`.

## What it does not do

The investment and portfolio decisions (transport-investment, knapsack-dp) do not read measured
line profit or payback from the store yet. Their events name the live-only fallback they took
(I0148). No decision reads the hub's own store. Without libduckdb the panel runs with
"store: off", and every stored input falls back and says so.

## Maturity

**stable** (definition: `README.md#what-a-status-means`).

- Complete: the bus, both sinks, the views (`tools/research/mcp-rhai/sql/views/`) and the query
  routes are on master. Milestone `planning-is-generated-from-recorded-history` is 7/7 DONE.
- Own check: the `mcp-panel` tests, which include `panel_store.rs` and `kernel_feed.rs`. CI runs
  them with the store feature against a pinned libduckdb (`.github/workflows/research-rust.yml`,
  job "Research Rust tests") on every master push and every pull request that touches
  `tools/research/mcp-rhai/**`.
- Last recorded pass: CI run 45538823227 at eed5cb5fc5 (master, 2026-11-25). The `mcp-panel`
  suite passed 425 tests, with 0 failed and 1 ignored.
