---
schema: knowledge/v1
id: the-majordomus-blackboard-is-written-by-announcing-on-a-board
kind: knowledge
class: fact
title: "The Majordomus blackboard has no write of its own: a process reaches it by announcing on a board as an MCP peer or through the mesh journal"
description: "Before forwarding anything (a bus digest, a status, a result) to the Majordomus blackboard so that other agent sessions see it."
status: verified
epistemics: observed
date: 2026-11-22
tags: [panel, bus, majordomus, blackboard]
about: tools/research/mcp-rhai/src/bin/mcp-bus-blackboard.rs
route: project
provenance:
  origin: authored
  derived_from:
    - file:tools/research/mcp-rhai/src/bin/mcp-bus-blackboard.rs
    - file:research/mcp-control.md
relations:
  - type: relates_to
    target: file:.ai/repo/adrs/0033-every-panel-event-goes-out-on-zeromq-and-into-a-duckdb-store.md
  - type: relates_to
    target: file:.ai/repo/knowledge/curated/the-panel-bus-carries-these-event-kinds.md
---

# Statement

Read in Majordomus 1.0.2 (source `prismatic-majordomus`, `apps/majordomus-cli/src/`), checked
against the running server of this checkout on 2026-11-18, and re-read at the v1.1.0 tag
(5bc2ddb10cc) on 2026-11-22:

- `capability/builtin/blackboard.rs` (ADR 0176) joins the boards (`peers.list`), the mesh journal
  and the monitor, and "writes nothing and keeps nothing". `blackboard.snapshot` has no write.
- The writes that reach it are `peers.announce` (`POST /api/v1/peers/announce`, or MCP
  `majordomus_announce`), which needs an MCP session; and the mesh journal: `POST
  /api/v1/mesh/sessions` (intent), `/mesh/beats` (phase), `/mesh/claims`, `/mesh/handovers`,
  `/mesh/reviews`. A session whose intent changes becomes a `session_changed` monitor signal
  (`monitor/mesh.rs`). A beat is suppressed as heartbeat noise.
- MCP over HTTP (`http/mcp.rs`) answers `POST /mcp` with JSON and an `Mcp-Session-Id`. It forgets
  a session after 90 s of silence, so a long-running client pings. The board keeps a departed
  peer's claims and drops the repeats of one client re-announcing the same named claims
  (`peers.rs:drop_repeated_departures`).
- Whether the mesh half carries anything depends on the machine. On 2026-11-18 `GET /api/v1/mesh`
  answered `active: false` ("this machine's key ... is not in trust.allow") and the blackboard listed
  its `mesh` and `monitor.mesh` sources as `absent`. On 2026-11-22 the same server answers
  `active: true` and both sources are `ok`.

# How to apply it

Use `mcp-bus-blackboard` as the pattern. Open an MCP HTTP session on the primary checkout's server
(`majordomus serve ensure --format json`), announce named claims with an empty scope (nothing to
collide with), and ping inside 90 s. The mesh journal reaches other machines only while this
machine's mesh is active; read `GET /api/v1/mesh` rather than assume it. Enrolling a machine's key
(`majordomus mesh enrol`, committed) is the owner's decision, not a tool's.

# Review

Authored from the read and the live run of issue-less task t-20261118022636-0218; awaiting the
owner's review.

# Evidence

Re-verified against Majordomus v1.1.0 (prismatic-majordomus tag v1.1.0 = 5bc2ddb10cc; installed majordomus 1.1.0) and this checkout at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- apps/majordomus-cli/src/capability/builtin/blackboard.rs:1-9 (ADR 0176) joins peers.list, the mesh journal and the monitor, "It writes nothing and keeps nothing".
- apps/majordomus-cli/src/capability/builtin/peers.rs:459-466 peers.announce, POST /api/v1/peers/announce, "Needs an MCP session"; :413 refuses a call with no peer identity.
- apps/majordomus-cli/src/capability/builtin/mesh.rs:2172-2289 the mesh journal writes: /api/v1/mesh/sessions, /beats, /claims, /handovers, /reviews.
- apps/majordomus-cli/src/monitor/mesh.rs:147 session_changed; :381 a beat is heartbeat noise.
- apps/majordomus-cli/src/http/mcp.rs:27 Mcp-Session-Id, :33 SESSION_IDLE_TIMEOUT 90 s.
- apps/majordomus-cli/src/peers.rs:902 drop_repeated_departures.
- tools/research/mcp-rhai/src/bin/mcp-bus-blackboard.rs:23-27 announces named claims with no scope via majordomus_announce.
- Live, 2026-11-22 ~09:46Z, http://127.0.0.1:53170: GET /api/v1/mesh answers active: true; GET /api/v1/blackboard lists mesh and monitor.mesh as ok. Revised today: the 2026-11-18 "mesh half is closed" claim is kept as history, not as current state.
