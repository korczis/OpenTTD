---
schema: feature/v1
id: embedded-mcp-server
kind: feature
title: Embedded loopback MCP server with approved, digest-bound mutation
short_title: MCP server
headline: An agent reads live game state over MCP and changes it only with an operator's approval of the exact arguments, through the command system.
summary: OPTION_PRISMATIC_MCP_SERVER (requires research instrumentation) serves Streamable HTTP on loopback from the game loop; every tool that changes the game waits for an operator's approval bound to its exact canonical arguments and is posted through the command system.
status: stable
weight: 50
areas: [agent-integration, observability]
audiences: [prismatic-integrator, coding-agent]
rules: [project.game-state-mutates-only-through-commands, project.research-code-is-gated-and-labelled, project.savegame-fields-are-version-gated]
adrs: [adr-0008]
related: [research-instrumentation, prismatic-bridge]
tags: [mcp, security, approval]
---

## What it does

The server binds 127.0.0.1 and ::1 only and services every request on the main thread from
`GameLoop()`. Every request needs a 256-bit bearer token that is never logged, and a request with a
foreign `Origin` is refused. Live game state is served as resources
(`research/prismatic-mcp/07-resource-catalog.md`), and the tools are listed in
`research/prismatic-mcp/08-tool-catalog.md`. Reading tools change nothing. Every tool that changes the
game is approval-gated: `openttd.cmd.<Name>`, `openttd.api.*` with execute, the `openttd.ai.*`
actions, `openttd.game.set_pause` and `openttd.decision.submit`. An operator approves the exact
canonical arguments in the console. A changed argument or a replay is refused (I0023), and an
approval dies with its session and its world (I0061). The approved command is posted through the
normal command path. In client mode the server acts only as its own company. Interface input
(`openttd.input.*`, the acting `openttd.ui.*` tools) runs only in a process armed with
`PRISMATIC_MCP_INPUT=1`. `openttd://mcp/status` and `prismatic_mcp status` report a
`capabilities` object derived from the tool registry and the arming state, and `read_only` is the
negation of every mutating capability (I0110). Every tool and resource is also a console command
derived from the catalogue (I0067). MCP reads leave the game deterministic: in the A/B of I0026,
16 of 16 game days had an identical `_random` state with and without read load.

## What it does not do

It exists only in a research build. CMake compiles none of it unless `OPTION_PRISMATIC_MCP_SERVER`
and `OPTION_RESEARCH_INSTRUMENTATION` are both ON, and a default build is checked to be free of it
(`tools/gate.sh research-free`). It listens on loopback only. An approval is given in the game
console. Otherwise, when its deadline expires, the standing verdict decides it. An approve verdict
applies only to a company that a registered approver plays, both when the approval was asked and
at the deadline. Registered approvers are known by their client key's fingerprint
(`openttd://approvals`, ed33855b1f). Naming an authenticated principal for every approval on every
surface is still open (I0182, milestone
`governed-access-is-authenticated-and-every-capability-reaches-every-surface`). How much of the
tool catalogue a live run actually exercises is counted by arrival, not by validated outcome
(I0183).

## Maturity

**stable** (definition: `README.md#what-a-status-means`).

- Complete: `src/prismatic_mcp/` is on master. Milestone `an-agent-reaches-the-game-over-mcp` is
  53/53 DONE.
- Own check: `openttd_test '[prismatic_mcp]'` (`src/tests/test_prismatic_mcp.cpp`). `tools/gate.sh
  research` runs it (`tools/gate.sh:745-748`), and `tools/gate.sh change` names that tier for every
  change to research-gated code. CI never builds the research options.
- Last recorded pass: at adab4ea012, which is in master's history: "All tests passed (1177
  assertions in 43 test cases)" on lundra (dedicated, research and MCP options ON). The result is
  recorded in PR #221 for I0110.
